use super::*;

pub type ConMeta = ConTyId;
pub type ConNode<T> = Node<T, ConMeta>;

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub enum ConTy {
    Prim(Prim),
    List(ConTyId),
    Tuple(Vec<ConTyId>),
    Record(BTreeMap<Ident, ConTyId>),
    Func(ConTyId, ConTyId),
    Data(DataId, Vec<ConTyId>),
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct ConTyId(usize);

pub type ConDef = (DefId, Vec<ConTyId>);

pub struct TyInsts<'a> {
    self_ty: Option<ConTyId>,
    gen: &'a [ConTyId],
}

pub struct ConContext {
    tys: Vec<ConTy>,
    ty_lookup: HashMap<ConTy, ConTyId>,
    defs: HashMap<ConDef, Option<ConExpr>>,
    entry: Option<ConDef>,
}

impl ConContext {
    pub fn from_ctx(hir: &Context) -> (Self, Vec<Error>) {
        let mut this = Self {
            tys: Vec::new(),
            ty_lookup: HashMap::default(),
            defs: HashMap::default(),
            entry: None,
        };

        let mut errors = Vec::new();

        let mut entries = hir.defs
            .iter()
            .filter_map(|(id, def)| def.attr
                .iter()
                .find(|attr| attr.as_str() == "main")
                .zip(Some((id, def))));

        if let Some((entry_attr, (id, main))) = entries.next() {
            if let Some((_, (_, second))) = entries.next() {
                errors.push(Error::MultipleEntryPoints(main.name.span(), second.name.span()));
            }

            let gen_scope = hir.tys.get_gen_scope(main.gen_scope);
            if gen_scope.len() == 0 {
                let main_def = (id, Vec::new());
                this.lower_def(hir, main_def);
                this.entry = Some((id, Vec::new()));
            } else {
                errors.push(Error::GenericEntryPoint(main.name.clone(), gen_scope.span, entry_attr.span()));
            }
        } else {
            errors.push(Error::NoEntryPoint(hir.root_span));
        }

        (this, errors)
    }

    pub fn entry_def(&self) -> ConDef {
        self.entry.clone().unwrap()
    }

    pub fn get_def(&self, def: &ConDef) -> &ConExpr {
        // Can't fail
        self.defs[def].as_ref().unwrap()
    }

    pub fn get_ty(&self, ty: ConTyId) -> &ConTy {
        &self.tys[ty.0]
    }

    pub fn insert_ty(&mut self, ty: ConTy) -> ConTyId {
        *self.ty_lookup
            .entry(ty.clone())
            .or_insert_with(|| {
                let id = ConTyId(self.tys.len());
                self.tys.push(ty);
                id
            })
    }

    pub fn concretize_ty(&mut self, hir: &Context, ty: TyId, ty_insts: &TyInsts) -> ConTyId {
        let cty = match hir.tys.get(ty) {
            Ty::Error(_) => panic!("Concretizable type cannot be an error"),
            Ty::Prim(prim) => ConTy::Prim(prim),
            Ty::List(item) => ConTy::List(self.concretize_ty(hir, item, ty_insts)),
            Ty::Tuple(fields) => ConTy::Tuple(fields
                .into_iter()
                .map(|field| self.concretize_ty(hir, field, ty_insts))
                .collect()),
            Ty::Record(fields) => ConTy::Record(fields
                .into_iter()
                .map(|(name, field)| (name, self.concretize_ty(hir, field, ty_insts)))
                .collect()),
            Ty::Func(i, o) => ConTy::Func(
                self.concretize_ty(hir, i, ty_insts),
                self.concretize_ty(hir, o, ty_insts),
            ),
            Ty::Data(data, args) => ConTy::Data(data, args
                .into_iter()
                .map(|arg| self.concretize_ty(hir, arg, ty_insts))
                .collect()),
            Ty::Gen(idx, _) => return ty_insts.gen[idx],
            Ty::SelfType => return ty_insts.self_ty.expect("Self type required during concretization but none was provided"),
        };

        self.insert_ty(cty)
    }

    // Returns (record_ty, field_ty, number_of_indirections)
    pub fn follow_field_access(&self, hir: &Context, mut ty: ConTyId, field: Ident) -> Option<(ConTyId, ConTyId, usize)> {
        let mut already_seen = Vec::new();

        loop {
            match self.get_ty(ty).clone() {
                ConTy::Data(data, args) => if already_seen.contains(&data) {
                    // We've already seen this data type, it must be recursive. Give up, it has no fields.
                    break None
                } else {
                    already_seen.push(data);
                    let data = hir.datas.get_data(data);
                    if data.cons.len() == 1 {
                        todo!();
                        // ty = self.concretize_ty(hir, data.cons[0].1, &TyInsts {
                        //     self_ty: None,
                        //     gen: &args,
                        // });
                    } else {
                        // Sum types have no fields
                        break None;
                    }
                },
                ConTy::Record(fields) => if let Some((_, field_ty)) = fields.iter().find(|(name, _)| **name == field) {
                    break Some((ty, *field_ty, already_seen.len()));
                } else {
                    // Record has no such field
                    break None;
                },
                _ => break None, // Only `Data` or `Record` can have fields
            }
        }
    }

    pub fn lower_def(&mut self, hir: &Context, def: ConDef) {
        if !self.defs.contains_key(&def) {
            self.defs.insert(def.clone(), None);

            let body = self.lower_expr(
                hir,
                hir.defs
                    .get(def.0)
                    .body
                    .as_ref()
                    .unwrap(),
                &TyInsts {
                    self_ty: None,
                    gen: &def.1,
                },
            );
            self.defs.insert(def, Some(body));
        }
    }

    pub fn lower_binding(&mut self, hir: &Context, binding: &TyBinding, ty_insts: &TyInsts) -> ConBinding {
        let pat = match &*binding.pat {
            hir::Pat::Error => panic!("Error pattern should not exist during concretization"),
            hir::Pat::Wildcard => hir::Pat::Wildcard,
            hir::Pat::Literal(litr) => hir::Pat::Literal(*litr),
            hir::Pat::Single(inner) => hir::Pat::Single(self.lower_binding(hir, inner, ty_insts)),
            hir::Pat::Add(lhs, rhs) => hir::Pat::Add(self.lower_binding(hir, lhs, ty_insts), rhs.clone()),
            hir::Pat::Tuple(fields) => hir::Pat::Tuple(fields
                .iter()
                .map(|field| self.lower_binding(hir, field, ty_insts))
                .collect()),
            hir::Pat::Record(fields) => hir::Pat::Record(fields
                .iter()
                .map(|(name, field)| (*name, self.lower_binding(hir, field, ty_insts)))
                .collect()),
            hir::Pat::ListExact(items) => hir::Pat::ListExact(items
                .iter()
                .map(|item| self.lower_binding(hir, item, ty_insts))
                .collect()),
            hir::Pat::ListFront(items, tail) => hir::Pat::ListFront(items
                .iter()
                .map(|item| self.lower_binding(hir, item, ty_insts))
                .collect(), tail.as_ref().map(|tail| self.lower_binding(hir, tail, ty_insts))),
            hir::Pat::Decons(data, variant, inner) => hir::Pat::Decons(data.clone(), *variant, self.lower_binding(hir, inner, ty_insts)),
        };

        ConNode::new(
            hir::Binding {
                pat: SrcNode::new(pat, binding.pat.span()),
                name: binding.name.clone(),
            },
            self.concretize_ty(hir, binding.meta().1, ty_insts),
        )
    }

    pub fn lower_expr(&mut self, hir: &Context, ty_expr: &TyExpr, ty_insts: &TyInsts) -> ConExpr {
        let expr = match ty_expr.inner() {
            hir::Expr::Error => panic!("Error expression should not exist during concretization"),
            hir::Expr::Literal(litr) => hir::Expr::Literal(*litr),
            hir::Expr::Local(local) => hir::Expr::Local(*local),
            hir::Expr::Global(x, args) => {
                let args = args
                    .iter()
                    .map(|arg| self.concretize_ty(hir, arg.1, ty_insts))
                    .collect::<Vec<_>>();
                self.lower_def(hir, (*x, args.clone()));
                hir::Expr::Global(*x, args)
            },
            hir::Expr::Tuple(fields) => hir::Expr::Tuple(fields
                .iter()
                .map(|field| self.lower_expr(hir, field, ty_insts))
                .collect()),
            hir::Expr::List(items) => hir::Expr::Tuple(items
                .iter()
                .map(|item| self.lower_expr(hir, item, ty_insts))
                .collect()),
            hir::Expr::ListFront(items, tail) => hir::Expr::ListFront(items
                .iter()
                .map(|item| self.lower_expr(hir, item, ty_insts))
                .collect(), self.lower_expr(hir, tail, ty_insts)),
            hir::Expr::Record(fields) => hir::Expr::Record(fields
                .iter()
                .map(|(name, field)| (name.clone(), self.lower_expr(hir, field, ty_insts)))
                .collect()),
            hir::Expr::Access(record, field) => hir::Expr::Access(self.lower_expr(hir, record, ty_insts), field.clone()),
            hir::Expr::Unary(op, a) => hir::Expr::Unary(op.clone(), self.lower_expr(hir, a, ty_insts)),
            hir::Expr::Binary(op, a, b) => hir::Expr::Binary(
                op.clone(),
                self.lower_expr(hir, a, ty_insts),
                self.lower_expr(hir, b, ty_insts),
            ),
            hir::Expr::Match(hidden_outer, pred, arms) => hir::Expr::Match(
                *hidden_outer,
                self.lower_expr(hir, pred, ty_insts),
                arms
                    .iter()
                    .map(|(binding, arm)| (
                        self.lower_binding(hir, binding, ty_insts),
                        self.lower_expr(hir, arm, ty_insts),
                    ))
                    .collect(),
            ),
            hir::Expr::Func(arg, body) => hir::Expr::Func(
                ConNode::new(**arg, self.concretize_ty(hir, arg.meta().1, ty_insts)),
                self.lower_expr(hir, body, ty_insts),
            ),
            hir::Expr::Apply(f, arg) => hir::Expr::Apply(
                self.lower_expr(hir, f, ty_insts),
                self.lower_expr(hir, arg, ty_insts),
            ),
            hir::Expr::Cons(data, variant, inner) => hir::Expr::Cons(data.clone(), *variant, self.lower_expr(hir, inner, ty_insts)),
            hir::Expr::ClassAccess(ty, class, field) => {
                let self_ty = self.concretize_ty(hir, *ty, ty_insts);
                let member = hir.classes
                    .lookup_member_concrete(hir, self, self_ty, *class)
                    .expect("Could not select member candidate");
                let member_gen_scope = hir.tys.get_gen_scope(member.gen_scope);

                fn derive_links(hir: &Context, ctx: &ConContext, member: TyId, ty: ConTyId, link_gen: &mut impl FnMut(usize, ConTyId)) {
                    match (hir.tys.get(member), ctx.get_ty(ty)) {
                        (Ty::Prim(x), ConTy::Prim(y)) => assert_eq!(x, *y),
                        (Ty::Gen(gen_idx, _), _) => link_gen(gen_idx, ty),
                        (Ty::List(x), ConTy::List(y)) => derive_links(hir, ctx, x, *y, link_gen),
                        (Ty::Tuple(xs), ConTy::Tuple(ys)) => xs
                            .into_iter()
                            .zip(ys.into_iter())
                            .for_each(|(x, y)| derive_links(hir, ctx, x, *y, link_gen)),
                        (Ty::Record(xs), ConTy::Record(ys)) => xs
                            .into_iter()
                            .zip(ys.into_iter())
                            .for_each(|((_, x), (_, y))| derive_links(hir, ctx, x, *y, link_gen)),
                        (Ty::Func(x_i, x_o), ConTy::Func(y_i, y_o)) => {
                            derive_links(hir, ctx, x_i, *y_i, link_gen);
                            derive_links(hir, ctx, x_o, *y_o, link_gen);
                        },
                        (Ty::Data(_, xs), ConTy::Data(_, ys)) => xs
                            .into_iter()
                            .zip(ys.into_iter())
                            .for_each(|(x, y)| derive_links(hir, ctx, x, *y, link_gen)),
                        (x, y) => todo!("{:?}", (x, y)),
                    }
                }
                let mut links = HashMap::new();
                derive_links(hir, self, member.member, self_ty, &mut |gen_idx, ty| { links.insert(gen_idx, ty); });
                let gen = (0..member_gen_scope.len())
                    .map(|idx| *links.get(&idx).expect("Generic type not mentioned in member"))
                    .collect::<Vec<_>>();

                let field = member
                    .field(**field)
                    .unwrap();
                self.lower_expr(hir, field, &TyInsts {
                    self_ty: Some(self_ty),
                    gen: &gen,
                }).into_inner()
            },
            hir::Expr::Debug(inner) => hir::Expr::Debug(self.lower_expr(hir, inner, ty_insts)),
        };

        ConNode::new(expr, self.concretize_ty(hir, ty_expr.meta().1, ty_insts))
    }
}