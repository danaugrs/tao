use super::*;

pub type TyMeta = (Span, TyId);
pub type TyNode<T> = Node<T, TyMeta>;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Prim {
    Nat,
    Int,
    Num,
    Bool,
    Char,
}

impl fmt::Display for Prim {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Prim::Nat => write!(f, "Nat"),
            Prim::Int => write!(f, "Int"),
            Prim::Num => write!(f, "Num"),
            Prim::Bool => write!(f, "Bool"),
            Prim::Char => write!(f, "Char"),
        }
    }
}

#[derive(Clone, Debug)]
pub enum Ty {
    Error,
    Prim(Prim),
    List(TyId),
    Tuple(Vec<TyId>),
    Record(HashMap<Ident, TyId>),
    Func(TyId, TyId),
    Data(DataId, Vec<TyId>),
    Gen(Ident, GenScopeId),
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct TyId(usize);

#[derive(Default)]
pub struct Types {
    tys: Vec<(Span, Ty)>,
    scopes: Vec<GenScope>,
}

impl Types {
    pub fn get_gen_scope(&self, scope: GenScopeId) -> &GenScope {
        &self.scopes[scope.0]
    }

    pub fn insert_gen_scope(&mut self, gen_scope: GenScope) -> GenScopeId {
        let id = GenScopeId(self.scopes.len());
        self.scopes.push(gen_scope);
        id
    }

    pub fn get(&self, ty: TyId) -> Ty {
        self.tys[ty.0].1.clone()
    }

    pub fn get_span(&self, ty: TyId) -> Span {
        self.tys[ty.0].0
    }

    pub fn insert(&mut self, span: Span, ty: Ty) -> TyId {
        let id = TyId(self.tys.len());
        self.tys.push((span, ty));
        id
    }

    pub fn display<'a>(&'a self, datas: &'a Datas, ty: TyId) -> impl fmt::Display + 'a {
        #[derive(Copy, Clone)]
        struct TyDisplay<'a> {
            types: &'a Types,
            datas: &'a Datas,
            ty: TyId,
            lhs_exposed: bool,
        }

        impl<'a> TyDisplay<'a> {
            fn with_ty(&self, ty: TyId, lhs_exposed: bool) -> Self {
                Self { ty, lhs_exposed, ..*self }
            }
        }

        impl<'a> fmt::Display for TyDisplay<'a> {
            fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
                match self.types.get(self.ty) {
                    Ty::Error => write!(f, "?"),
                    Ty::Prim(prim) => write!(f, "{}", prim),
                    Ty::List(item) => write!(f, "[{}]", self.with_ty(item, false)),
                    Ty::Tuple(items) => write!(f, "({}{})", items
                        .iter()
                        .map(|item| format!("{}", self.with_ty(*item, false)))
                        .collect::<Vec<_>>()
                        .join(", "), if items.len() == 1 { "," } else { "" }),
                    Ty::Record(fields) => write!(f, "{{ {} }}", fields
                        .into_iter()
                        .map(|(name, field)| format!("{}: {}", name, self.with_ty(field, false)))
                        .collect::<Vec<_>>()
                        .join(", ")),
                    Ty::Func(i, o) if self.lhs_exposed => write!(f, "({} -> {})", self.with_ty(i, true), self.with_ty(o, self.lhs_exposed)),
                    Ty::Func(i, o) => write!(f, "{} -> {}", self.with_ty(i, true), self.with_ty(o, self.lhs_exposed)),
                    Ty::Data(name, params) if self.lhs_exposed && params.len() > 0 => write!(f, "({}{})", self.datas.get_data(name).name, params
                        .iter()
                        .map(|param| format!(" {}", self.with_ty(*param, true)))
                        .collect::<String>()),
                    Ty::Data(name, params) => write!(f, "{}{}", self.datas.get_data(name).name, params
                        .iter()
                        .map(|param| format!(" {}", self.with_ty(*param, true)))
                        .collect::<String>()),
                    Ty::Gen(name, _scope) => write!(f, "{}", name),
                }
            }
        }

        TyDisplay {
            types: self,
            datas,
            ty,
            lhs_exposed: false,
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct GenScopeId(usize);

pub struct GenScope {
    types: Vec<Ident>,
}

impl GenScope {
    pub fn from_ast(generics: &ast::Generics) -> Self {
        Self {
            types: generics.tys
                .iter()
                .map(|ty| **ty)
                .collect(),
        }
    }

    pub fn len(&self) -> usize { self.types.len() }

    pub fn index(&self, name: Ident) -> Option<usize> {
        self.types.iter().position(|ty| ty == &name)
    }
}
