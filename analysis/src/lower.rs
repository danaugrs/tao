use super::*;

// TODO: use `ToHir`?
fn litr_ty_info(litr: &ast::Literal, infer: &mut Infer, span: Span) -> TyInfo {
    match litr {
        ast::Literal::Nat(_) => TyInfo::Prim(ty::Prim::Nat),
        ast::Literal::Int(_) => TyInfo::Prim(ty::Prim::Int),
        ast::Literal::Real(x) => TyInfo::Prim(ty::Prim::Real),
        ast::Literal::Bool(_) => TyInfo::Prim(ty::Prim::Bool),
        ast::Literal::Char(_) => TyInfo::Prim(ty::Prim::Char),
        ast::Literal::Str(_) => TyInfo::List(infer.insert(span, TyInfo::Prim(ty::Prim::Char))),
    }
}

pub enum Scope<'a> {
    Empty,
    Recursive(SrcNode<Ident>, TyVar, DefId, Vec<(Span, TyVar)>),
    Binding(&'a Scope<'a>, SrcNode<Ident>, TyVar),
    Many(&'a Scope<'a>, &'a [(SrcNode<Ident>, TyVar)]),
    Basin(&'a Scope<'a>, EffectVar),
}

impl<'a> Scope<'a> {
    pub fn empty() -> Self { Self::Empty }

    fn with(&self, name: SrcNode<Ident>, ty: TyVar) -> Scope<'_> {
        Scope::Binding(self, name, ty)
    }

    fn with_many<'b>(&'b self, many: &'b [(SrcNode<Ident>, TyVar)]) -> Scope<'b> {
        Scope::Many(self, many)
    }

    fn with_basin<'b>(&'b self, basin: EffectVar) -> Scope<'b> {
        Scope::Basin(self, basin)
    }

    // bool = is_local
    fn find(&self, infer: &mut Infer, span: Span, name: &Ident) -> Option<(TyVar, Option<(DefId, Vec<(Span, TyVar)>)>)> {
        match self {
            Self::Empty => None,
            Self::Recursive(def, ty, def_id, tys) => if &**def == name {
                Some((infer.try_reinstantiate(span, *ty), Some((*def_id, tys.clone()))))
            } else {
                None
            },
            Self::Binding(_, local, ty) if &**local == name => Some((*ty, None)),
            Self::Binding(parent, _, _) => parent.find(infer, span, name),
            Self::Many(parent, locals) => if let Some((_, ty)) = locals.iter().find(|(local, _)| &**local == name) {
                Some((*ty, None))
            } else {
                parent.find(infer, span, name)
            },
            Self::Basin(parent, _) => parent.find(infer, span, name),
        }
    }

    fn last_basin(&self) -> Option<EffectVar> {
        match self {
            Self::Empty => None,
            Self::Recursive(_, _, _, _) => None,
            Self::Binding(parent, _, _) => parent.last_basin(),
            Self::Many(parent, _) => parent.last_basin(),
            Self::Basin(_, eff) => Some(*eff),
        }
    }
}

pub trait ToHir: Sized {
    type Output;

    fn to_hir(self: &SrcNode<Self>, infer: &mut Infer, scope: &Scope) -> InferNode<Self::Output>;
}

// Enforce the obligations of a kind's generics
// Fails if the number of parameters doesn't match the number of generic types in the scope
fn enforce_generic_obligations(
    infer: &mut Infer,
    gen_scope: GenScopeId,
    params: &[TyVar],
    use_span: Span,
    item_span: Span,
) -> Result<(), ()> {
    let gen_scope = infer.ctx().tys.get_gen_scope(gen_scope);

    if gen_scope.len() != params.len() {
        let err = Error::WrongNumberOfGenerics(
            use_span,
            params.len(),
            if gen_scope.len() == 0 {
                item_span
            } else {
                gen_scope.span
            },
            gen_scope.len(),
        );
        infer.ctx_mut().emit(err);
        Err(())
    } else {
        // Enforce obligations from data type
        let mut obls = Vec::new();
        for idx in 0..gen_scope.len() {
            for obl in gen_scope
                .get(idx)
                .obligations()
            {
                match &**obl {
                    Obligation::MemberOf(class) => obls.push((idx, *class, obl.span())),
                }
            }
        }
        for (idx, class, span) in obls {
            infer.make_impl(params[idx], class, use_span, Vec::new(), span);
        }
        Ok(())
    }
}

impl ToHir for ast::Type {
    type Output = ();

    fn to_hir(self: &SrcNode<Self>, infer: &mut Infer, scope: &Scope) -> InferNode<()> {
        let info = match &**self {
            ast::Type::Error => TyInfo::Error(ErrorReason::Unknown),
            ast::Type::Unknown => TyInfo::Unknown(None),
            ast::Type::Universe => TyInfo::Prim(Prim::Universe),
            ast::Type::List(item) => TyInfo::List(item.to_hir(infer, scope).meta().1),
            ast::Type::Tuple(items) => TyInfo::Tuple(items
                .iter()
                .map(|item| item.to_hir(infer, scope).meta().1)
                .collect()),
            ast::Type::Record(fields) => TyInfo::Record(fields
                .iter()
                .map(|(name, field)| (**name, field.to_hir(infer, scope).meta().1))
                .collect()),
            ast::Type::Func(i, o) => TyInfo::Func(i.to_hir(infer, scope).meta().1, o.to_hir(infer, scope).meta().1),
            ast::Type::Data(name, params) => match (name.as_str(), params.len()) {
                ("Self", 0) => if let Some(var) = infer.self_type() {
                    TyInfo::Ref(var)
                } else {
                    infer.ctx_mut().emit(Error::SelfNotValidHere(name.span()));
                    TyInfo::Error(ErrorReason::Invalid)
                },
                ("Nat", 0) => TyInfo::Prim(Prim::Nat),
                ("Int", 0) => TyInfo::Prim(Prim::Int),
                ("Real", 0) => TyInfo::Prim(Prim::Real),
                ("Bool", 0) => TyInfo::Prim(Prim::Bool),
                ("Char", 0) => TyInfo::Prim(Prim::Char),
                _ => {
                    let params = params
                        .iter()
                        .map(|param| param.to_hir(infer, scope).meta().1)
                        .collect::<Vec<_>>();

                    if let Some((scope, (gen_idx, gen_ty))) = infer
                        .gen_scope()
                        .and_then(|scope| Some((scope, infer.ctx().tys.get_gen_scope(scope).find(**name)?)))
                    {
                        TyInfo::Gen(gen_idx, scope, gen_ty.name.span())
                    } else if let Some(alias_id) = infer.ctx().datas.lookup_alias(**name) {
                        if let Some(alias) = infer.ctx().datas.get_alias(alias_id) {
                            let alias_gen_scope = infer.ctx().tys.get_gen_scope(alias.gen_scope);
                            if alias_gen_scope.len() != params.len() {
                                let err = Error::WrongNumberOfGenerics(
                                    self.span(),
                                    params.len(),
                                    alias_gen_scope.span,
                                    alias_gen_scope.len(),
                                );
                                infer.ctx_mut().emit(err);
                                TyInfo::Error(ErrorReason::Unknown)
                            } else {
                                let (alias_ty, alias_gen_scope) = (alias.ty, alias.gen_scope);
                                let get_gen = |index, scope, ctx: &Context| {
                                    params[index]
                                };

                                // Bit messy, makes sure that we don't accidentally infer a bad type backwards
                                let inner_ty_actual = infer.instantiate(alias_ty, self.span(), &get_gen, None);
                                let inner_ty = infer.unknown(self.span());
                                infer.make_flow(inner_ty_actual, inner_ty, EqInfo::from(self.span()));

                                TyInfo::Ref(inner_ty)
                            }
                        } else {
                            let err_ty = infer.insert(self.span(), TyInfo::Error(ErrorReason::Unknown));
                            infer.emit(InferError::RecursiveAlias(alias_id, err_ty, name.span()));
                            TyInfo::Ref(err_ty)
                        }
                    } else if let Some(data) = infer.ctx().datas.lookup_data(**name) {
                        match enforce_generic_obligations(
                            infer,
                            infer.ctx().datas.name_gen_scope(**name),
                            &params,
                            self.span(),
                            infer.ctx().datas.get_data_span(data),
                        ) {
                            Ok(()) => TyInfo::Data(data, params),
                            Err(()) => TyInfo::Error(ErrorReason::Unknown),
                        }
                    } else {
                        infer.ctx_mut().emit(Error::NoSuchData(name.clone()));
                        TyInfo::Error(ErrorReason::Invalid)
                    }
                },
            },
            ast::Type::Assoc(inner, assoc) => {
                let inner = inner.to_hir(infer, scope);
                let assoc_ty = infer.unknown(self.span());
                infer.make_class_assoc(inner.meta().1, assoc.clone(), assoc_ty, self.span());
                TyInfo::Ref(assoc_ty)
            },
            ast::Type::Effect(name, params, out) => {
                let params = params
                    .iter()
                    .map(|param| param.to_hir(infer, scope).meta().1)
                    .collect::<Vec<_>>();

                let out = out.to_hir(infer, scope).meta().1;

                if let Some(eff_id) = infer.ctx().effects.lookup(**name) {
                    let eff = infer.ctx().effects.get_decl(eff_id);
                    let eff_gen_scope = eff.gen_scope;
                    let eff_span = eff.name.span();
                    match enforce_generic_obligations(
                        infer,
                        eff_gen_scope,
                        &params,
                        self.span(),
                        eff_span,
                    ) {
                        Ok(()) => {
                            let eff = infer.insert_effect(self.span(), EffectInfo::Known(eff_id, params));
                            TyInfo::Effect(eff, out)
                        },
                        Err(()) => TyInfo::Error(ErrorReason::Unknown),
                    }
                } else {
                    infer.ctx_mut().emit(Error::NoSuchEffect(name.clone()));
                    TyInfo::Error(ErrorReason::Invalid)
                }
            },
        };

        InferNode::new((), (self.span(), infer.insert(self.span(), info)))
    }
}

impl ToHir for ast::Binding {
    type Output = hir::Binding<InferMeta>;

    fn to_hir(self: &SrcNode<Self>, infer: &mut Infer, scope: &Scope) -> InferNode<Self::Output> {
        let (info, pat) = match &*self.pat {
            ast::Pat::Error => (TyInfo::Error(ErrorReason::Unknown), hir::Pat::Error),
            ast::Pat::Wildcard => (TyInfo::Unknown(None), hir::Pat::Wildcard),
            ast::Pat::Literal(litr) => {
                let ty_info = litr_ty_info(litr, infer, self.pat.span());
                (TyInfo::Ref(infer.insert(self.span(), ty_info)), match litr {
                    ast::Literal::Nat(x) => hir::Pat::Literal(ast::Literal::Nat(*x)),
                    ast::Literal::Int(x) => hir::Pat::Literal(ast::Literal::Int(*x)),
                    ast::Literal::Real(x) => hir::Pat::Literal(ast::Literal::Real(*x)),
                    ast::Literal::Bool(x) => hir::Pat::Literal(ast::Literal::Bool(*x)),
                    ast::Literal::Char(c) => hir::Pat::Literal(ast::Literal::Char(*c)),
                    ast::Literal::Str(s) => {
                        // Bit of a hack, we do this because equality of literals is not supported in the MIR, so a
                        // string pattern desugars to a list pattern.
                        let c_ty = infer.insert(self.span(), TyInfo::Prim(Prim::Char));
                        hir::Pat::ListExact(s
                            .chars()
                            .map(|c| InferNode::new(hir::Binding::from_pat(SrcNode::new(hir::Pat::Literal(ast::Literal::Char(c)), self.span())), (self.span(), c_ty)))
                            .collect())
                    },
                })
            },
            ast::Pat::Single(inner) => {
                let binding = inner.to_hir(infer, scope);
                // TODO: don't use `Ref` to link types
                (TyInfo::Ref(binding.meta().1), hir::Pat::Single(binding))
            },
            ast::Pat::Binary(op, lhs, rhs) => {
                let lhs = lhs.to_hir(infer, scope);
                match (&**rhs, &**op) {
                    (ast::Literal::Nat(rhs_nat), ast::BinaryOp::Add) => {
                        let nat = infer.insert(rhs.span(), TyInfo::Prim(Prim::Nat));
                        infer.make_flow(lhs.meta().1, nat, EqInfo::new(self.span(), format!("Only natural numbers support arithmetic patterns")));
                        (TyInfo::Ref(nat), hir::Pat::Add(lhs, SrcNode::new(*rhs_nat, rhs.span())))
                    },
                    (_, _) => {
                        let ty_info = litr_ty_info(rhs, infer, self.pat.span());
                        let rhs_ty = infer.insert(rhs.span(), ty_info);
                        infer.emit(InferError::PatternNotSupported(lhs.meta().1, op.clone(), rhs_ty, self.span()));
                        (TyInfo::Error(ErrorReason::Unknown), hir::Pat::Error)
                    },
                }
            },
            ast::Pat::Tuple(items) => {
                let items = items
                    .iter()
                    .map(|item| item.to_hir(infer, scope))
                    .collect::<Vec<_>>();
                (TyInfo::Tuple(items
                    .iter()
                    .map(|item| item.meta().1)
                    .collect()), hir::Pat::Tuple(items))
            },
            ast::Pat::Record(fields) => {
                let fields = fields
                    .iter()
                    .map(|(name, field)| (**name, field.to_hir(infer, scope)))
                    .collect::<BTreeMap<_, _>>();
                (TyInfo::Record(fields
                    .iter()
                    .map(|(name, field)| (*name, field.meta().1))
                    .collect()), hir::Pat::Record(fields))
            },
            ast::Pat::ListExact(items) => {
                let item_ty = infer.unknown(self.pat.span());
                let items = items
                    .iter()
                    .map(|item| {
                        let item = item.to_hir(infer, scope);
                        infer.make_flow(item.meta().1, item_ty, item.meta().0);
                        item
                    })
                    .collect::<Vec<_>>();
                (TyInfo::List(item_ty), hir::Pat::ListExact(items))
            },
            ast::Pat::ListFront(items, tail) => {
                let item_ty = infer.unknown(self.pat.span());
                let items = items
                    .iter()
                    .map(|item| {
                        let item = item.to_hir(infer, scope);
                        infer.make_flow(item.meta().1, item_ty, item.meta().0);
                        item
                    })
                    .collect::<Vec<_>>();
                let tail = tail.as_ref().map(|tail| {
                    let tail = tail.to_hir(infer, scope);
                    let ty = infer.insert(tail.meta().0, TyInfo::List(item_ty));
                    infer.make_flow(tail.meta().1, ty, tail.meta().0);
                    tail
                });
                (TyInfo::List(item_ty), hir::Pat::ListFront(items, tail))
            },
            ast::Pat::Deconstruct(name, inner) => if let Some(data) = infer.ctx().datas.lookup_cons(**name) {
                let gen_scope_id = infer.ctx().datas.get_data(data).gen_scope;
                let gen_scope = infer.ctx().tys.get_gen_scope(gen_scope_id);
                let generics_count = gen_scope.len();
                let generic_tys = (0..generics_count)
                    .map(|i| gen_scope.get(i).name.span())
                    .collect::<Vec<_>>()
                    .into_iter()
                    .map(|origin| infer.insert(self.span(), TyInfo::Unknown(Some(origin))))
                    .collect::<Vec<_>>();

                if let Err(()) = enforce_generic_obligations(
                    infer,
                    gen_scope_id,
                    &generic_tys,
                    self.span(),
                    infer.ctx().datas.get_data_span(data),
                ) {
                    // Only fails if generic count doesn't match, but they're from the same source of truth!
                    unreachable!();
                }

                let inner_ty = infer
                    .ctx()
                    .datas
                    .get_data(data)
                    .cons
                    .iter()
                    .find(|(cons, _)| **cons == **name)
                    .unwrap()
                    .1;

                // Recreate type in context
                let inner_ty = {
                    let data = infer.ctx().datas.get_data(data);
                    let data_gen_scope = data.gen_scope;
                    let get_gen = |index, _, ctx: &Context| generic_tys[index];

                    // Bit messy, makes sure that we don't accidentally infer a bad type backwards
                    let inner_ty_actual = infer.instantiate(inner_ty, inner.span(), &get_gen, None);
                    let inner_ty = infer.unknown(self.span());
                    infer.make_flow(inner_ty_actual, inner_ty, EqInfo::from(self.span()));

                    inner_ty
                };

                let inner = inner.to_hir(infer, scope);
                infer.make_flow(inner_ty, inner.meta().1, self.span());

                (TyInfo::Data(data, generic_tys), hir::Pat::Decons(SrcNode::new(data, self.span()), **name, inner))
            } else {
                infer.ctx_mut().emit(Error::NoSuchCons(name.clone()));
                // TODO: Don't use a hard, preserve inner expression
                (TyInfo::Error(ErrorReason::Unknown), hir::Pat::Error)
            },
        };

        let ty = infer.insert(self.span(), info);

        if let Some(ty_hint) = &self.ty {
            let hint = ty_hint.to_hir(infer, scope);
            infer.make_flow(ty, hint.meta().1, self.span());
        }

        InferNode::new(hir::Binding {
            pat: SrcNode::new(pat, self.pat.span()),
            name: self.name.clone(),
        }, (self.span(), ty))
    }
}

fn instantiate_def(def_id: DefId, span: Span, infer: &mut Infer, span_override: Option<Span>, inst_span: Span) -> (TyInfo, hir::Expr<InferMeta>) {
    let scope = infer.ctx().tys.get_gen_scope(infer.ctx().defs.get(def_id).gen_scope);
    let generics_count = scope.len();
    let generic_tys = (0..generics_count)
        .map(|i| TyInfo::Unknown(Some(scope.get(i).name.span())))
        .collect::<Vec<_>>()
        .into_iter()
        .map(|info| (span, infer.insert(span, info)))
        .collect::<Vec<_>>();

    // Enforce class obligations
    for (idx, (span, ty)) in generic_tys.iter().enumerate() {
        let scope = infer.ctx().tys.get_gen_scope(infer.ctx().defs.get(def_id).gen_scope);
        for obl in scope
            .get(idx)
            .obligations()
            .to_vec()
        {
            match &*obl {
                Obligation::MemberOf(class) => infer.make_impl(*ty, *class, obl.span(), Vec::new(), inst_span),
            }
        }
    }

    // Recreate type in context
    let def = infer.ctx().defs.get(def_id);
    let def_gen_scope = def.gen_scope;
    let def_name = def.name.clone();
    let get_gen = |index: usize, _, ctx: &Context| generic_tys[index].1;
    let ty = if let Some(body_ty) = def.ty_hint
        .or_else(|| def.body
            .as_ref()
            .map(|body| body.meta().1))
    {
        // Bit messy, makes sure that we don't accidentally infer a bad type backwards
        let def_ty_actual = infer.instantiate(body_ty, span_override, &get_gen, None);
        let def_ty = infer.unknown(span);
        infer.make_flow(def_ty_actual, def_ty, EqInfo::from(inst_span));
        Some(def_ty)
    } else {
        None
    };

    if let Some(ty) = ty {
        (TyInfo::Ref(ty), hir::Expr::Global((def_id, generic_tys)))
    } else {
        infer.ctx_mut().emit(Error::DefTypeNotSpecified(def_name.span(), span, *def_name));
        (TyInfo::Error(ErrorReason::Unknown), hir::Expr::Error)
    }
}

impl ToHir for ast::Expr {
    type Output = hir::Expr<InferMeta>;

    fn to_hir(self: &SrcNode<Self>, infer: &mut Infer, scope: &Scope) -> InferNode<Self::Output> {
        let mut span = self.span();
        let (info, expr) = match &**self {
            ast::Expr::Error => (TyInfo::Error(ErrorReason::Unknown), hir::Expr::Error),
            ast::Expr::Literal(litr) => {
                let ty_info = litr_ty_info(litr, infer, span);
                (TyInfo::Ref(infer.insert(span, ty_info)), hir::Expr::Literal(*litr))
            },
            ast::Expr::LangDef(def) => {
                let def = match def {
                    ast::LangDef::IoUnit => infer.ctx().defs.lang.io_unit.unwrap(),
                    ast::LangDef::IoBind => infer.ctx().defs.lang.io_bind.unwrap(),
                };

                instantiate_def(def, self.span(), infer, Some(self.span()), self.span())
            },
            ast::Expr::Local(local) => {
                if let Some((ty, rec)) = scope.find(infer, self.span(), &local) {
                    if let Some((def_id, gens)) = rec {
                        (TyInfo::Ref(ty), hir::Expr::Global((def_id, gens)))
                    } else {
                        (TyInfo::Ref(ty), hir::Expr::Local(*local))
                    }
                } else if let Some(def_id) = infer.ctx().defs.lookup(*local) {
                    instantiate_def(def_id, self.span(), infer, None, self.span())
                } else {
                    infer.ctx_mut().emit(Error::NoSuchLocal(SrcNode::new(*local, self.span())));
                    (TyInfo::Error(ErrorReason::Unknown), hir::Expr::Error)
                }
            },
            ast::Expr::Tuple(items) => {
                let items = items
                    .iter()
                    .map(|item| item.to_hir(infer, scope))
                    .collect::<Vec<_>>();
                let tys = items
                    .iter()
                    .map(|item| item.meta().1)
                    .collect();
                (TyInfo::Tuple(tys), hir::Expr::Tuple(items))
            },
            ast::Expr::List(items, tails) => {
                let item_ty = infer.unknown(self.span());
                let list_ty = infer.insert(self.span(), TyInfo::List(item_ty));

                let items = items
                    .iter()
                    .map(|item| {
                        let item = item.to_hir(infer, scope);
                        infer.make_flow(item.meta().1, item_ty, item.meta().0);
                        item
                    })
                    .collect::<Vec<_>>();

                let tails = tails
                    .iter()
                    .map(|tail| {
                        let tail = tail.to_hir(infer, scope);
                        infer.make_flow(tail.meta().1, list_ty, tail.meta().0);
                        tail
                    })
                    .collect();

                (TyInfo::Ref(list_ty), hir::Expr::List(items, tails))
            },
            ast::Expr::Record(fields) => {
                let fields = fields
                    .iter()
                    .map(|(name, field)| (name.clone(), field.to_hir(infer, scope)))
                    .collect::<Vec<_>>();
                let tys = fields
                    .iter()
                    .map(|(name, field)| (**name, field.meta().1))
                    .collect();
                (TyInfo::Record(tys), hir::Expr::Record(fields))
            },
            ast::Expr::Access(record, field) => {
                let record = record.to_hir(infer, scope);
                let field_ty = infer.unknown(self.span());
                infer.make_access(record.meta().1, field.clone(), field_ty);
                // TODO: don't use `Ref` to link types
                (TyInfo::Ref(field_ty), hir::Expr::Access(record, field.clone()))
            },
            ast::Expr::Unary(op, a) => if let ast::UnaryOp::Propagate = &**op {
                let a = a.to_hir(infer, scope);
                let out_ty = infer.unknown(self.span());

                let eff = if let Some(eff) = scope.last_basin() {
                    eff
                } else {
                    infer.ctx_mut().errors.push(Error::NoBasin(op.span()));
                    infer.unknown_effect(a.meta().0)
                };

                let eff_obj_ty = infer.insert(a.meta().0, TyInfo::Effect(eff, out_ty));
                infer.make_flow(a.meta().1, eff_obj_ty, EqInfo::from(op.span()));

                (TyInfo::Ref(out_ty), hir::Expr::Intrinsic(SrcNode::new(Intrinsic::Propagate, op.span()), vec![a]))
            } else {
                let a = a.to_hir(infer, scope);
                let output_ty = infer.unknown(self.span());

                let func = infer.insert(op.span(), TyInfo::Func(a.meta().1, output_ty));

                let (class, field) = match &**op {
                    ast::UnaryOp::Not => (infer.ctx().classes.lang.not, SrcNode::new(Ident::new("not"), self.span())),
                    ast::UnaryOp::Neg => (infer.ctx().classes.lang.neg, SrcNode::new(Ident::new("neg"), op.span())),
                    ast::UnaryOp::Propagate => unreachable!(), // handled above
                };
                let class = infer.make_class_field_known(a.meta().1, field.clone(), class, func, self.span());

                (TyInfo::Ref(output_ty), hir::Expr::Apply(InferNode::new(hir::Expr::ClassAccess(*a.meta(), class, field), (op.span(), func)), a))
            },
            ast::Expr::Binary(op, a, b) => {
                let a = a.to_hir(infer, scope);
                let b = b.to_hir(infer, scope);
                let output_ty = infer.unknown(self.span());

                let lang_op = match &**op {
                    ast::BinaryOp::Eq => Some((infer.ctx().classes.lang.eq, SrcNode::new(Ident::new("eq"), self.span()))),
                    _ => None,
                };

                if let Some((class, field)) = lang_op {
                    let func2 = infer.insert(op.span(), TyInfo::Func(b.meta().1, output_ty));
                    let func = infer.insert(op.span(), TyInfo::Func(a.meta().1, func2));
                    infer.make_flow(a.meta().1, b.meta().1, EqInfo::from(self.span()));
                    infer.make_flow(b.meta().1, a.meta().1, EqInfo::from(self.span()));

                    let class = infer.make_class_field_known(a.meta().1, field.clone(), class, func, self.span());

                    let expr = hir::Expr::Apply(
                        InferNode::new(hir::Expr::Apply(
                            InferNode::new(hir::Expr::ClassAccess(*a.meta(), class, field), (op.span(), func)),
                            a,
                        ), (self.span(), func)),
                        b,
                    );

                    (TyInfo::Ref(output_ty), expr)
                } else {
                    infer.make_binary(op.clone(), a.meta().1, b.meta().1, output_ty);
                    (TyInfo::Ref(output_ty), hir::Expr::Binary(op.clone(), a, b))
                }
            },
            ast::Expr::Let(bindings, then) => {
                fn fold<'a>(then: &SrcNode<ast::Expr>, bindings: &'a mut impl Iterator<Item = &'a (SrcNode<ast::Binding>, SrcNode<ast::Expr>)>, infer: &mut Infer, scope: &Scope) -> InferExpr {
                    match bindings.next() {
                        Some((binding, val)) => {
                            let binding = binding.to_hir(infer, scope);
                            let val = val.to_hir(infer, scope);
                            infer.make_flow(val.meta().1, binding.meta().1, val.meta().0);
                            let then = fold(then, bindings, infer, &scope.with_many(&binding.get_binding_tys()));
                            let span = binding.meta().0;
                            let ty = then.meta().1; // TODO: Make a TyInfo::Ref?
                            InferNode::new(hir::Expr::Match(
                                false,
                                val,
                                vec![(binding, then)],
                            ), (span, ty))
                        },
                        None => then.to_hir(infer, scope),
                    }
                }

                let expr = fold(then, &mut bindings.iter(), infer, scope);
                span = expr.meta().0;
                (TyInfo::Ref(expr.meta().1), expr.into_inner())
            },
            ast::Expr::Match(preds, arms) => {
                let mut is_err = false;
                for arm in arms {
                    if arm.0.len() != preds.len() {
                        infer.ctx_mut().emit(Error::WrongNumberOfParams(arm.0.span(), arm.0.len(), preds.span(), preds.len()));
                        is_err = true;
                    }
                }

                if is_err {
                    (TyInfo::Error(ErrorReason::Unknown), hir::Expr::Error)
                } else {
                    let pred = tupleify_expr(preds, infer, scope);

                    let output_ty = infer.unknown(self.span());

                    let arms = arms
                        .iter()
                        .map(|(bindings, body)| {
                            let binding = tupleify_binding(bindings, infer, scope);
                            infer.make_flow(pred.meta().1, binding.meta().1, binding.meta().0);
                            let body = body.to_hir(infer, &scope.with_many(&binding.get_binding_tys()));
                            infer.make_flow(body.meta().1, output_ty, EqInfo::new(self.span(), format!("Branches must produce compatible values")));
                            (binding, body)
                        })
                        .collect();

                    (TyInfo::Ref(output_ty), hir::Expr::Match(true, pred, arms))
                }
            },
            ast::Expr::If(pred, a, b) => {
                let pred_ty = infer.insert(pred.span(), TyInfo::Prim(Prim::Bool));
                let output_ty = infer.unknown(self.span());
                let pred = pred.to_hir(infer, scope);
                infer.make_flow(pred.meta().1, pred_ty, EqInfo::new(self.span(), format!("Conditions must be booleans")));
                let a = a.to_hir(infer, scope);
                let b = b.to_hir(infer, scope);
                infer.make_flow(a.meta().1, output_ty, EqInfo::new(self.span(), format!("Branches must produce compatible values")));
                infer.make_flow(b.meta().1, output_ty, EqInfo::new(self.span(), format!("Branches must produce compatible values")));
                let arms = vec![
                    (InferNode::new(hir::Binding::from_pat(SrcNode::new(hir::Pat::Literal(ast::Literal::Bool(true)), pred.meta().0)), *pred.meta()), a),
                    (InferNode::new(hir::Binding::from_pat(SrcNode::new(hir::Pat::Literal(ast::Literal::Bool(false)), pred.meta().0)), *pred.meta()), b),
                ];
                (TyInfo::Ref(output_ty), hir::Expr::Match(false, pred, arms))
            },
            ast::Expr::Func(arms) => {
                // TODO: Don't always refuse 0-branch functions? Can they be useful with never types?
                if let Some(first_arm) = arms.first() {
                    let mut is_err = false;
                    for arm in arms.iter() {
                        if arm.0.len() != first_arm.0.len() {
                            infer.ctx_mut().emit(Error::WrongNumberOfParams(arm.0.span(), arm.0.len(), first_arm.0.span(), first_arm.0.len()));
                            is_err = true;
                        }
                    }

                    if is_err {
                        (TyInfo::Error(ErrorReason::Unknown), hir::Expr::Error)
                    } else {
                        let output_ty = infer.unknown(if arms.len() == 1 {
                            arms[0].1.span()
                        } else {
                            arms.span()
                        });

                        let pseudos = (0..first_arm.0.len())
                            .map(|i| {
                                let name = SrcNode::new(Ident::new(i), self.span());
                                let ty = infer.unknown(first_arm.0[i].span());
                                (name, ty)
                            })
                            .collect::<Vec<_>>();

                        let pred = tupleify_expr(&SrcNode::new(pseudos
                            .iter()
                            .map(|(pseudo, _)| SrcNode::new(ast::Expr::Local(**pseudo), pseudo.span()))
                            .collect(), self.span()), infer, &scope.with_many(&pseudos.clone()));

                        let arms = arms
                            .iter()
                            .map(|(bindings, body)| {
                                let binding = tupleify_binding(bindings, infer, scope);
                                infer.make_flow(pred.meta().1, binding.meta().1, binding.meta().0);
                                let body = body.to_hir(infer, &scope.with_many(&binding.get_binding_tys()));
                                infer.make_flow(body.meta().1, output_ty, EqInfo::new(self.span(), format!("Branches must produce compatible values")));
                                (binding, body)
                            })
                            .collect();

                        let pred_meta = *pred.meta();
                        let f = pseudos
                            .into_iter()
                            .rev()
                            .fold(
                                InferNode::new(hir::Expr::Match(true, pred, arms), (self.span(), output_ty)),
                                |body, (pseudo, pseudo_ty)| {
                                    let f = infer.insert(self.span(), TyInfo::Func(pseudo_ty, body.meta().1));
                                    InferNode::new(hir::Expr::Func(InferNode::new(*pseudo, pred_meta), body), (self.span(), f))
                                },
                            );

                        (TyInfo::Ref(f.meta().1), f.into_inner())
                    }
                } else {
                    infer.ctx_mut().emit(Error::NoBranches(self.span()));
                    (TyInfo::Error(ErrorReason::Unknown), hir::Expr::Error)
                }
            },
            ast::Expr::Apply(f, param) => {
                let f = f.to_hir(infer, scope);
                let param = param.to_hir(infer, scope);
                let input_ty = infer.unknown(param.meta().0);
                let output_ty = infer.unknown(self.span());
                let func = infer.insert(f.meta().0, TyInfo::Func(input_ty, output_ty));
                infer.make_flow(f.meta().1, func, EqInfo::new(self.span(), format!("Only functions are callable")));
                infer.make_flow(param.meta().1, input_ty, EqInfo::new(param.meta().0, format!("Functions may only be called with compatible arguments")));

                (TyInfo::Ref(output_ty), hir::Expr::Apply(f, param))
            },
            ast::Expr::Cons(name, inner) => if let Some(data) = infer.ctx().datas.lookup_cons(**name) {
                let gen_scope = infer.ctx().tys.get_gen_scope(infer.ctx().datas.get_data(data).gen_scope);
                let generics_count = gen_scope.len();
                let generic_tys = (0..generics_count)
                    .map(|i| gen_scope.get(i).name.span())
                    .collect::<Vec<_>>()
                    .into_iter()
                    .map(|origin| infer.insert(self.span(), TyInfo::Unknown(Some(origin))))
                    .collect::<Vec<_>>();

                if let Err(()) = enforce_generic_obligations(
                    infer,
                    infer.ctx().datas.get_data(data).gen_scope,
                    &generic_tys,
                    self.span(),
                    infer.ctx().datas.get_data_span(data),
                ) {
                    // Only fails if generic count doesn't match, but they're from the same source of truth!
                    unreachable!();
                }

                let inner_ty = infer
                    .ctx()
                    .datas
                    .get_data(data)
                    .cons
                    .iter()
                    .find(|(cons, _)| **cons == **name)
                    .unwrap()
                    .1;

                // Recreate type in context
                let inner_ty = {
                    let data = infer.ctx().datas.get_data(data);
                    let data_gen_scope = data.gen_scope;
                    let get_gen = |index, _, ctx: &Context| generic_tys[index];

                    // Bit messy, makes sure that we don't accidentally infer a bad type backwards
                    let inner_ty_actual = infer.instantiate(inner_ty, name.span(), &get_gen, None);
                    let inner_ty = infer.unknown(self.span());
                    infer.make_flow(inner_ty, inner_ty_actual, EqInfo::from(self.span()));

                    inner_ty
                };

                let inner = inner.to_hir(infer, scope);
                infer.make_flow(inner.meta().1, inner_ty, self.span());

                (TyInfo::Data(data, generic_tys), hir::Expr::Cons(SrcNode::new(data, name.span()), **name, inner))
            } else {
                infer.ctx_mut().emit(Error::NoSuchCons(name.clone()));
                // TODO: Don't use a hard, preserve inner expression
                (TyInfo::Error(ErrorReason::Unknown), hir::Expr::Error)
            },
            ast::Expr::ClassAccess(ty, field) => {
                let ty = ty.to_hir(infer, scope);
                let field_ty = infer.unknown(field.span());
                let class = infer.make_class_field(ty.meta().1, field.clone(), field_ty, ty.meta().0);
                (TyInfo::Ref(field_ty), hir::Expr::ClassAccess(*ty.meta(), class, field.clone()))
            },
            ast::Expr::Intrinsic(name, args) => {
                let mut args = args
                    .iter()
                    .map(|arg| arg.to_hir(infer, scope))
                    .collect::<Vec<_>>();
                match name.as_str() {
                    "type_name" if args.len() == 1 => {
                        // Takes an empty list
                        let item = infer.unknown(args[0].meta().0);
                        let list = infer.insert(args[0].meta().0, TyInfo::List(item));
                        infer.make_flow(args[0].meta().1, list, EqInfo::from(name.span()));
                        // Produces a string
                        let c = infer.insert(name.span(), TyInfo::Prim(Prim::Char));
                        (TyInfo::List(c), hir::Expr::Intrinsic(SrcNode::new(Intrinsic::TypeName, name.span()), args))
                    },
                    "neg_nat" if args.len() == 1 => {
                        let a = &args[0];
                        let nat = infer.insert(a.meta().0, TyInfo::Prim(Prim::Nat));
                        infer.make_flow(args[0].meta().1, nat, EqInfo::from(name.span()));
                        (TyInfo::Prim(Prim::Int), hir::Expr::Intrinsic(SrcNode::new(Intrinsic::NegNat, name.span()), args))
                    },
                    "neg_int" if args.len() == 1 => {
                        let a = &args[0];
                        let nat = infer.insert(a.meta().0, TyInfo::Prim(Prim::Int));
                        infer.make_flow(args[0].meta().1, nat, EqInfo::from(name.span()));
                        (TyInfo::Prim(Prim::Int), hir::Expr::Intrinsic(SrcNode::new(Intrinsic::NegInt, name.span()), args))
                    },
                    "neg_real" if args.len() == 1 => {
                        let a = &args[0];
                        let nat = infer.insert(a.meta().0, TyInfo::Prim(Prim::Real));
                        infer.make_flow(args[0].meta().1, nat, EqInfo::from(name.span()));
                        (TyInfo::Prim(Prim::Real), hir::Expr::Intrinsic(SrcNode::new(Intrinsic::NegReal, name.span()), args))
                    },
                    "eq_char" if args.len() == 2 => {
                        let a = &args[0];
                        let b = &args[1];
                        let char_ = infer.insert(a.meta().0, TyInfo::Prim(Prim::Char));
                        infer.make_flow(args[0].meta().1, char_, EqInfo::from(name.span()));
                        infer.make_flow(args[1].meta().1, char_, EqInfo::from(name.span()));
                        (TyInfo::Prim(Prim::Bool), hir::Expr::Intrinsic(SrcNode::new(Intrinsic::EqChar, name.span()), args))
                    },
                    "eq_nat" if args.len() == 2 => {
                        let a = &args[0];
                        let b = &args[1];
                        let nat = infer.insert(a.meta().0, TyInfo::Prim(Prim::Nat));
                        infer.make_flow(args[0].meta().1, nat, EqInfo::from(name.span()));
                        infer.make_flow(args[1].meta().1, nat, EqInfo::from(name.span()));
                        (TyInfo::Prim(Prim::Bool), hir::Expr::Intrinsic(SrcNode::new(Intrinsic::EqNat, name.span()), args))
                    },
                    "go" if args.len() == 2 => if let Some(go_data) = infer.ctx().datas.lang.go {
                        let c = args[1].meta().1;
                        let r = infer.unknown(self.span());
                        let ret = infer.insert(args[0].meta().0, TyInfo::Data(go_data, vec![c, r]));
                        let f = infer.insert(args[0].meta().0, TyInfo::Func(c, ret));
                        infer.make_flow(args[0].meta().1, f, EqInfo::from(name.span()));
                        (TyInfo::Ref(r), hir::Expr::Intrinsic(SrcNode::new(Intrinsic::Go, name.span()), args))
                    } else {
                        (TyInfo::Error(ErrorReason::Unknown), hir::Expr::Error)
                    },
                    "print" if args.len() == 2 => {
                        let a = &args[0];
                        let b = &args[1];

                        let universe = infer.insert(a.meta().0, TyInfo::Prim(Prim::Universe));
                        infer.make_flow(a.meta().1, universe, EqInfo::from(name.span()));

                        let c = infer.insert(b.meta().0, TyInfo::Prim(Prim::Char));
                        let s = infer.insert(b.meta().0, TyInfo::List(c));
                        infer.make_flow(b.meta().1, s, EqInfo::from(name.span()));

                        (TyInfo::Prim(Prim::Universe), hir::Expr::Intrinsic(SrcNode::new(Intrinsic::Print, name.span()), args))
                    },
                    "input" if args.len() == 1 => {
                        let a = &args[0];

                        let universe = infer.insert(a.meta().0, TyInfo::Prim(Prim::Universe));
                        infer.make_flow(a.meta().1, universe, EqInfo::from(name.span()));

                        let c = infer.insert(self.span(), TyInfo::Prim(Prim::Char));
                        let s = infer.insert(self.span(), TyInfo::List(c));

                        (TyInfo::Tuple(vec![universe, s]), hir::Expr::Intrinsic(SrcNode::new(Intrinsic::Input, name.span()), args))
                    },
                    "len_list" if args.len() == 1 => {
                        let item = infer.unknown(args[0].meta().0);
                        let list = infer.insert(args[0].meta().0, TyInfo::List(item));
                        infer.make_flow(args[0].meta().1, list, EqInfo::from(name.span()));
                        (TyInfo::Prim(Prim::Nat), hir::Expr::Intrinsic(SrcNode::new(Intrinsic::LenList, name.span()), args))
                    },
                    "skip_list" if args.len() == 2 => {
                        let item = infer.unknown(args[0].meta().0);
                        let list = infer.insert(args[0].meta().0, TyInfo::List(item));
                        infer.make_flow(args[0].meta().1, list, EqInfo::from(name.span()));
                        let nat = infer.insert(args[1].meta().0, TyInfo::Prim(Prim::Nat));
                        infer.make_flow(args[1].meta().1, nat, EqInfo::from(name.span()));
                        (TyInfo::Ref(list), hir::Expr::Intrinsic(SrcNode::new(Intrinsic::SkipList, name.span()), args))
                    },
                    "trim_list" if args.len() == 2 => {
                        let item = infer.unknown(args[0].meta().0);
                        let list = infer.insert(args[0].meta().0, TyInfo::List(item));
                        infer.make_flow(args[0].meta().1, list, EqInfo::from(name.span()));
                        let nat = infer.insert(args[1].meta().0, TyInfo::Prim(Prim::Nat));
                        infer.make_flow(args[1].meta().1, nat, EqInfo::from(name.span()));
                        (TyInfo::Ref(list), hir::Expr::Intrinsic(SrcNode::new(Intrinsic::TrimList, name.span()), args))
                    },
                    "suspend" if args.len() == 1 => {
                        let a = &args[0];
                        let out = infer.unknown(self.span());

                        let eff = if let Some(eff) = scope.last_basin() {
                            eff
                        } else {
                            infer.ctx_mut().errors.push(Error::NoBasin(name.span()));
                            infer.unknown_effect(a.meta().0)
                        };

                        infer.make_effect_send_recv(eff, a.meta().1, out, self.span());
                        (TyInfo::Ref(out), hir::Expr::Suspend(eff, args.remove(0)))
                    },
                    _ => {
                        infer.ctx_mut().emit(Error::InvalidIntrinsic(name.clone()));
                        (TyInfo::Error(ErrorReason::Invalid), hir::Expr::Error)
                    },
                }
            },
            ast::Expr::Update(record, fields) => {
                let record = record.to_hir(infer, scope);
                let fields = fields
                    .iter()
                    .map(|(name, field)| {
                        let field = field.to_hir(infer, scope);
                        infer.make_update(record.meta().1, name.clone(), field.meta().1);
                        (name.clone(), field)
                    })
                    .collect();

                (TyInfo::Ref(record.meta().1), hir::Expr::Update(record, fields))
            },
            ast::Expr::Block(init, last) => {
                let eff = infer.unknown_effect(self.span());

                // Collect effects into this basin
                let scope = scope.with_basin(eff);
                let init = init
                    .iter()
                    .map(|stmt| stmt.to_hir(infer, &scope))
                    .collect::<Vec<_>>();
                let last = last.to_hir(infer, &scope);

                let last_meta = *last.meta();
                let chain = init
                    .into_iter()
                    .rev()
                    .fold(last, |then, before| {
                        let before_meta = *before.meta();
                        let then_meta = *then.meta();
                        InferNode::new(hir::Expr::Match(false, before, vec![
                            (InferNode::new(hir::Binding {
                                pat: SrcNode::new(hir::Pat::Wildcard, before_meta.0),
                                name: None,
                            }, before_meta), then),
                        ]), then_meta)
                    });

                (TyInfo::Effect(eff, last_meta.1), hir::Expr::Basin(eff, chain))
            },
            ast::Expr::Handle { expr, eff_name, eff_args, send, recv } => {
                let expr = expr.to_hir(infer, &scope);
                let send = send.to_hir(infer, &scope);
                let recv = recv.to_hir(infer, &scope.with_many(&send.get_binding_tys()));

                let eff_args = eff_args
                    .iter()
                    .map(|arg| arg.to_hir(infer, scope).meta().1)
                    .collect::<Vec<_>>();

                let out_ty = infer.unknown(self.span());

                if let Some(eff_id) = infer.ctx().effects.lookup(**eff_name) {
                    let eff = infer.ctx().effects.get_decl(eff_id);
                    let eff_gen_scope = eff.gen_scope;
                    let eff_span = eff.name.span();
                    match enforce_generic_obligations(
                        infer,
                        eff_gen_scope,
                        &eff_args,
                        self.span(),
                        eff_span,
                    ) {
                        Ok(()) => {
                            let eff = infer.insert_effect(self.span(), EffectInfo::Known(eff_id, eff_args));
                            infer.make_effect_send_recv(eff, send.meta().1, recv.meta().1, eff_name.span());
                            let eff_obj_ty = infer.insert(expr.meta().0, TyInfo::Effect(eff, out_ty));
                            infer.make_flow(expr.meta().1, eff_obj_ty, EqInfo::from(self.span()));

                            let recv_meta = *recv.meta();
                            (TyInfo::Ref(out_ty), hir::Expr::Handle {
                                expr,
                                eff,
                                send: InferNode::new(Ident::new("send"), *send.meta()),
                                recv: InferNode::new(hir::Expr::Match(
                                    false,
                                    InferNode::new(hir::Expr::Local(Ident::new("send")), *send.meta()),
                                    vec![(send, recv)],
                                ), recv_meta),
                            })
                        },
                        Err(()) => (TyInfo::Error(ErrorReason::Unknown), hir::Expr::Error),
                    }
                } else {
                    infer.ctx_mut().emit(Error::NoSuchEffect(eff_name.clone()));
                    (TyInfo::Error(ErrorReason::Invalid), hir::Expr::Error) // TODO: Can we avoid making this entire node an error?
                }
            },
        };

        InferNode::new(expr, (span, infer.insert(span, info)))
    }
}

// Desugar a list of expressions into a tuple
fn tupleify_expr(items: &SrcNode<Vec<SrcNode<ast::Expr>>>, infer: &mut Infer, scope: &Scope) -> InferExpr {
    let hir_items = items
        .iter()
        .map(|item| item.to_hir(infer, scope))
        .collect::<Vec<_>>();
    let tuple_ty = infer.insert(items.span(), TyInfo::Tuple(hir_items
        .iter()
        .map(|item| item.meta().1)
        .collect()));
    InferNode::new(hir::Expr::Tuple(hir_items), (items.span(), tuple_ty))
}

// Desugar a list of bindings into a tuple
fn tupleify_binding(items: &SrcNode<Vec<SrcNode<ast::Binding>>>, infer: &mut Infer, scope: &Scope) -> InferBinding {
    let hir_items = items
        .iter()
        .map(|item| item.to_hir(infer, scope))
        .collect::<Vec<_>>();
    let tuple_ty = infer.insert(items.span(), TyInfo::Tuple(hir_items
        .iter()
        .map(|item| item.meta().1)
        .collect()));
    let binding = hir::Binding {
        pat: SrcNode::new(hir::Pat::Tuple(hir_items), items.span()),
        name: None,
    };
    InferNode::new(binding, (items.span(), tuple_ty))
}
