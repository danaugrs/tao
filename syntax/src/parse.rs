use super::*;

pub trait Parser<T> = chumsky::Parser<Token, T, Error = Error> + Clone;

pub fn literal_parser() -> impl Parser<ast::Literal> {
    filter_map(|span, token| match token {
        Token::Nat(x) => Ok(ast::Literal::Nat(x)),
        Token::Num(x) => Ok(ast::Literal::Num(x.parse().expect("Valid number could not be parsed as f64"))),
        Token::Bool(x) => Ok(ast::Literal::Bool(x)),
        Token::Char(x) => Ok(ast::Literal::Char(x)),
        Token::Str(x) => Ok(ast::Literal::Str(x)),
        token => Err(Error::expected_input_found(span, None, Some(token))),
    })
}

pub fn term_ident_parser() -> impl Parser<ast::Ident> {
    filter_map(|span, token| match token {
        Token::TermIdent(x) => Ok(x),
        token => Err(Error::expected_input_found(span, None, Some(token))),
    })
}

pub fn type_ident_parser() -> impl Parser<ast::Ident> {
    filter_map(|span, token| match token {
        Token::TypeIdent(x) => Ok(x),
        token => Err(Error::expected_input_found(span, None, Some(token))),
    })
}

pub fn nat_parser() -> impl Parser<u64> {
    filter_map(|span, token| match token {
        Token::Nat(x) => Ok(x),
        token => Err(Error::expected_input_found(span, None, Some(token))),
    })
}

pub fn bool_parser() -> impl Parser<bool> {
    filter_map(|span, token| match token {
        Token::Bool(x) => Ok(x),
        token => Err(Error::expected_input_found(span, None, Some(token))),
    })
}

pub fn nested_parser<'a, T: 'a>(parser: impl Parser<T> + 'a, delimiter: Delimiter, f: impl Fn(Span) -> T + Clone + 'a) -> impl Parser<T> + 'a {
    parser
        .delimited_by(Token::Open(delimiter), Token::Close(delimiter))
        .recover_with(nested_delimiters(
            Token::Open(delimiter), Token::Close(delimiter),
            [
                (Token::Open(Delimiter::Paren), Token::Close(Delimiter::Paren)),
                (Token::Open(Delimiter::Brack), Token::Close(Delimiter::Brack)),
                (Token::Open(Delimiter::Brace), Token::Close(Delimiter::Brace)),
            ],
            f,
        ))
        .boxed()
}

pub fn type_parser() -> impl Parser<ast::Type> {
    recursive(|ty| {
        let data = type_ident_parser() // TODO: Replace with `data_item_parser` when ready
            .map_with_span(SrcNode::new)
            .map(|data_name| ast::Type::Data(data_name, Vec::new()));

        let list = nested_parser(
            ty.clone()
                .map_with_span(SrcNode::new)
                .map(Some),
            Delimiter::Brack,
            |_| None,
        )
            .map(|ty| ty.map(ast::Type::List).unwrap_or(ast::Type::Error));

        let tuple = nested_parser(
            ty.clone()
                .map_with_span(SrcNode::new)
                .separated_by(just(Token::Comma))
                .allow_trailing()
                .then_ignore(just(Token::Comma).or_not())
                .map(Some),
            Delimiter::Paren,
            |_| None,
        )
            .map(|tys| tys.map(ast::Type::Tuple).unwrap_or(ast::Type::Error));

        let record = nested_parser(
            term_ident_parser()
                .map_with_span(SrcNode::new)
                .then_ignore(just(Token::Op(Op::Eq)))
                .then(ty.clone().map_with_span(SrcNode::new))
                .separated_by(just(Token::Comma))
                .allow_trailing()
                .then_ignore(just(Token::Comma).or_not())
                .map(Some)
                .boxed(),
            Delimiter::Brace,
            |_| None,
        )
            .map(|tys| tys.map(ast::Type::Record).unwrap_or(ast::Type::Error));

        let unknown = just(Token::Question)
            .map(|_| ast::Type::Unknown);

        let paren_ty = nested_parser(
            ty.clone().map(Some),
            Delimiter::Paren,
            |_| None,
        )
            .map(|ty| ty.unwrap_or(ast::Type::Error));

        let atom = paren_ty
            .or(data)
            .or(list)
            .or(tuple)
            .or(record)
            .or(unknown)
            .map_with_span(SrcNode::new)
            .boxed();

        let data = type_ident_parser() // TODO: Replace with `data_item_parser` when ready
            .map_with_span(SrcNode::new)
            .then(atom.clone().repeated())
            .map(|(data, params)| ast::Type::Data(data, params))
            .map_with_span(SrcNode::new)
            .or(atom)
            .boxed();

        data.clone()
            .then(just(Token::Op(Op::RArrow))
                .ignore_then(ty.clone().map_with_span(SrcNode::new))
                .repeated())
            .foldl(|i, o| {
                let span = i.span().union(o.span());
                SrcNode::new(ast::Type::Func(i, o), span)
            })
            .or(data)
            .map(|ty| ty.into_inner())
    })
}

pub fn ty_hint_parser() -> impl Parser<Option<SrcNode<ast::Type>>> {
    just(Token::Colon)
        .ignore_then(type_parser()
            .map_with_span(SrcNode::new))
        .or_not()
}

pub fn binding_parser() -> impl Parser<ast::Binding> {
    recursive(move |binding| {
        let wildcard = just(Token::Wildcard)
            .map_with_span(|_, span| SrcNode::new(ast::Pat::Wildcard, span));

        let litr = literal_parser()
            .map_with_span(|litr, span| SrcNode::new(ast::Pat::Literal(litr), span));

        let paren_expr = nested_parser(
            binding.clone()
                .map(Some),
            Delimiter::Paren,
            |_| None,
        )
            .map(|x| x.map(ast::Pat::Single).unwrap_or(ast::Pat::Error))
            .map_with_span(SrcNode::new);

        let tuple = nested_parser(
            binding.clone()
                .separated_by(just(Token::Comma))
                .allow_trailing()
                .map(Some),
            Delimiter::Paren,
            |_| None,
        )
            .map(|x| x.map(ast::Pat::Tuple).unwrap_or(ast::Pat::Error))
            .map_with_span(SrcNode::new);

        let record = nested_parser(
            term_ident_parser()
                .map_with_span(SrcNode::new)
                .then(ty_hint_parser())
                .then(just(Token::Op(Op::Eq))
                    .ignore_then(binding.clone())
                    .or_not())
                .map_with_span(|((field, ty), binding), span| {
                    let binding = binding.unwrap_or_else(|| SrcNode::new(ast::Binding {
                        pat: SrcNode::new(ast::Pat::Wildcard, field.span()),
                        name: Some(field.clone()),
                        ty,
                    }, span));

                    (field, binding)
                })
                .separated_by(just(Token::Comma))
                .allow_trailing()
                .then_ignore(just(Token::Comma).or_not())
                .map(Some)
                .boxed(),
            Delimiter::Brace,
            |_| None,
        )
            .map(|x| x.map(ast::Pat::Record).unwrap_or(ast::Pat::Error))
            .map_with_span(SrcNode::new);

        let list = nested_parser(
            binding.clone()
                .separated_by(just(Token::Comma))
                .allow_trailing()
                .then(just(Token::Op(Op::Ellipsis))
                    .ignore_then(binding.clone().or_not())
                    .or_not())
                .map(Some)
                .boxed(),
            Delimiter::Brack,
            |_| None,
        )
            .map(|x| x
                .map(|(items, tail)| match tail {
                    Some(tail) => ast::Pat::ListFront(items, tail),
                    None => ast::Pat::ListExact(items),
                })
                .unwrap_or(ast::Pat::Error))
            .map_with_span(SrcNode::new);

        let deconstruct = type_ident_parser() // TODO: Replace with `data_item_parser` when ready
            .map_with_span(SrcNode::new)
            .then(binding.or_not())
            .map_with_span(|(data, inner), span| {
                let inner = inner.unwrap_or_else(|| SrcNode::new(ast::Binding {
                    pat: SrcNode::new(ast::Pat::Tuple(Vec::new()), data.span()),
                    name: None,
                    ty: None,
                }, data.span()));

                SrcNode::new(ast::Pat::Deconstruct(data, inner), span)
            });

        let pat = wildcard
            .or(litr)
            .or(deconstruct)
            .or(paren_expr)
            .or(tuple)
            .or(record)
            .or(list)
            .boxed();

        // Bound pattern
        term_ident_parser()
            .map_with_span(SrcNode::new)
            .then_ignore(just(Token::Tilde))
            .then(pat.clone())
            .map(|(binding, pat)| (pat, Some(binding)))
            // Unbound pattern
            .or(pat.map(|pat| (pat, None)))
            // Ident
            .or(term_ident_parser().map_with_span(|name, span| (
                SrcNode::new(ast::Pat::Wildcard, span),
                Some(SrcNode::new(name, span)),
            )))
            .then(ty_hint_parser())
            .map_with_span(|((pat, name), ty), span| SrcNode::new(ast::Binding {
                pat,
                name,
                ty,
            }, span))
            .boxed()
    })
        .map(|expr| expr.into_inner())
        .labelled("pattern")
}

pub fn expr_parser() -> impl Parser<ast::Expr> {
    recursive(|expr| {
        let litr = literal_parser().map(ast::Expr::Literal);
        let ident = term_ident_parser().map(ast::Expr::Local);

        let paren_exp_list = nested_parser(
            expr
                .clone()
                .map_with_span(SrcNode::new)
                .separated_by(just(Token::Comma))
                .allow_trailing()
                .map(Some),
            Delimiter::Paren,
            |_| None,
        );

        let tuple = paren_exp_list
            .clone()
            .map(|x| x.map(ast::Expr::Tuple).unwrap_or(ast::Expr::Error))
            .labelled("tuple");

        let record = nested_parser(
            term_ident_parser()
                .map_with_span(SrcNode::new)
                .then(just(Token::Colon)
                    .ignore_then(expr.clone())
                    .map_with_span(SrcNode::new)
                    .or_not())
                .map(|(field, val)| match val {
                    Some(val) => (field, val),
                    None => {
                        let val = SrcNode::new(ast::Expr::Local(*field), field.span());
                        (field, val)
                    },
                })
                .separated_by(just(Token::Comma))
                .allow_trailing()
                .map(ast::Expr::Record)
                .boxed(),
            Delimiter::Brace,
            |_| ast::Expr::Error,
        )
            .labelled("record");

        let list = nested_parser(
            expr
                .clone()
                .map_with_span(SrcNode::new)
                .separated_by(just(Token::Comma))
                .allow_trailing()
                .map(Some),
            Delimiter::Brack,
            |_| None,
        )
            .map(|x| x.map(ast::Expr::List).unwrap_or(ast::Expr::Error))
            .labelled("list");

        let branch = binding_parser()
            .map_with_span(SrcNode::new)
            .separated_by(just(Token::Comma))
            .allow_trailing()
            .map_with_span(SrcNode::new)
            .then_ignore(just(Token::Op(Op::RFlow)))
            .then(expr
                .clone()
                .map_with_span(SrcNode::new))
            .boxed();

        let branches = branch.clone()
            .separated_by(just(Token::Pipe))
            .allow_leading();

        let func = just(Token::Fn)
            .ignore_then(branches.clone())
            .map(ast::Expr::Func);

        let cons = type_ident_parser()
            .map_with_span(SrcNode::new)
            .then(expr.clone()
                .map_with_span(SrcNode::new)
                .or_not())
            .map(|(cons, expr)| {
                let span = cons.span();
                ast::Expr::Cons(
                    cons,
                    expr.unwrap_or_else(|| SrcNode::new(ast::Expr::Tuple(Vec::new()), span)),
                )
            });

        let let_ = just(Token::Let)
            .ignore_then(binding_parser().map_with_span(SrcNode::new)
                .then_ignore(just(Token::Op(Op::Eq)))
                .then(expr.clone().map_with_span(SrcNode::new))
                .separated_by(just(Token::Comma))
                .allow_trailing())
            .then_ignore(just(Token::In))
            .then(expr.clone().map_with_span(SrcNode::new))
            .map(|(bindings, then)| ast::Expr::Let(bindings, then))
            .boxed();

        let if_ = just(Token::If)
            .ignore_then(expr.clone().map_with_span(SrcNode::new))
            .then_ignore(just(Token::Then))
            .then(expr.clone().map_with_span(SrcNode::new))
            .then_ignore(just(Token::Else))
            .then(expr.clone().map_with_span(SrcNode::new))
            .map(|((pred, a), b)| ast::Expr::If(pred, a, b))
            .boxed();

        let match_ = just(Token::Match)
            .ignore_then(expr.clone().map_with_span(SrcNode::new)
                .separated_by(just(Token::Comma))
                .allow_trailing()
                .map_with_span(SrcNode::new))
            .then_ignore(just(Token::In))
            .then(branches)
            .map(|(inputs, branches)| ast::Expr::Match(inputs, branches))
            .boxed();

        let atom = litr
            .or(ident)
            .or(nested_parser(expr, Delimiter::Paren, |_| ast::Expr::Error))
            .or(tuple)
            .or(record)
            .or(list)
            .or(let_)
            .or(if_)
            .or(match_)
            .or(func)
            .or(cons)
            .map_with_span(SrcNode::new)
            .boxed();

        // Apply direct (a pattern like `f(arg)` more eagerly binds than a simple application chain
        let direct = atom
            .then(paren_exp_list.clone().or_not())
            .map_with_span(|(expr, args), span| match args {
                Some(Some(args)) => {
                    let arg_count = args.len();
                    args
                        .into_iter()
                        .enumerate()
                        .fold(expr, |f, (i, arg)| {
                            let span = if i == arg_count - 1 {
                                span
                            } else {
                                f.span().union(arg.span())
                            };
                            SrcNode::new(ast::Expr::Apply(f, arg), span)
                        })
                },
                Some(None) => SrcNode::new(ast::Expr::Error, span),
                None => expr,
            });

        enum Chain {
            Field(SrcNode<ast::Ident>),
            Infix(SrcNode<ast::Expr>),
            Apply(Option<Vec<SrcNode<ast::Expr>>>, Span),
        }

        let chain = just(Token::Op(Op::Dot))
            .ignore_then(term_ident_parser().map_with_span(SrcNode::new))
            .map(Chain::Field)
            .or(just(Token::Colon).ignore_then(direct.clone())
                .map(Chain::Infix))
            .or(paren_exp_list
                .map_with_span(|args, span| Chain::Apply(args, span)))
            .boxed();

        let chained = direct
            .then(chain.repeated())
            .foldl(|expr, chain| match chain {
                Chain::Field(field) => {
                    let span = expr.span().union(field.span());
                    SrcNode::new(ast::Expr::Access(expr, field), span)
                },
                Chain::Infix(f) => {
                    let span = expr.span().union(f.span());
                    SrcNode::new(ast::Expr::Apply(f, expr), span)
                },
                Chain::Apply(None, span) => SrcNode::new(ast::Expr::Error, expr.span()),
                Chain::Apply(Some(args), outer_span) => {
                    let arg_count = args.len();
                    args
                        .into_iter()
                        .enumerate()
                        .fold(expr, |f, (i, arg)| {
                            let span = if i == arg_count - 1 {
                                outer_span
                            } else {
                                f.span().union(arg.span())
                            };
                            SrcNode::new(ast::Expr::Apply(f, arg), span)
                        })
                },
            })
            .boxed();

        // Unary
        let op = just(Token::Op(Op::Sub)).to(ast::UnaryOp::Neg)
            .or(just(Token::Op(Op::Not)).to(ast::UnaryOp::Not))
            .map_with_span(SrcNode::new);
        let unary = op.repeated()
            .then(chained)
            .foldr(|op, expr| {
                let span = op.span().union(expr.span());
                SrcNode::new(ast::Expr::Unary(op, expr), span)
            })
            .boxed();

        // Product
        let op = just(Token::Op(Op::Mul)).to(ast::BinaryOp::Mul)
            .or(just(Token::Op(Op::Div)).to(ast::BinaryOp::Div))
            .or(just(Token::Op(Op::Rem)).to(ast::BinaryOp::Rem))
            .map_with_span(SrcNode::new);
        let product = unary.clone()
            .then(op.then(unary).repeated())
            .foldl(|a, (op, b)| {
                let span = a.span().union(b.span());
                SrcNode::new(ast::Expr::Binary(op, a, b), span)
            })
            .boxed();

        // Sum
        let op = just(Token::Op(Op::Add)).to(ast::BinaryOp::Add)
            .or(just(Token::Op(Op::Sub)).to(ast::BinaryOp::Sub))
            .map_with_span(SrcNode::new);
        let sum = product.clone()
            .then(op.then(product).repeated())
            .foldl(|a, (op, b)| {
                let span = a.span().union(b.span());
                SrcNode::new(ast::Expr::Binary(op, a, b), span)
            })
            .boxed();

        // List joining
        let op = just(Token::Op(Op::Join)).to(ast::BinaryOp::Join)
            .map_with_span(SrcNode::new);
        let join = sum.clone()
            .then(op.then(sum).repeated())
            .foldl(|a, (op, b)| {
                let span = a.span().union(b.span());
                SrcNode::new(ast::Expr::Binary(op, a, b), span)
            })
            .boxed();

        // Comparison
        let op = just(Token::Op(Op::Less)).to(ast::BinaryOp::Less)
            .or(just(Token::Op(Op::LessEq)).to(ast::BinaryOp::LessEq))
            .or(just(Token::Op(Op::More)).to(ast::BinaryOp::More))
            .or(just(Token::Op(Op::MoreEq)).to(ast::BinaryOp::MoreEq))
            .or(just(Token::Op(Op::Eq)).to(ast::BinaryOp::Eq))
            .or(just(Token::Op(Op::NotEq)).to(ast::BinaryOp::NotEq))
            .map_with_span(SrcNode::new);
        let comparison = join.clone()
            .then(op.then(join).repeated())
            .foldl(|a, (op, b)| {
                let span = a.span().union(b.span());
                SrcNode::new(ast::Expr::Binary(op, a, b), span)
            })
            .boxed();

        // Logical
        let op = just(Token::Op(Op::And)).to(ast::BinaryOp::And)
            .or(just(Token::Op(Op::Or)).to(ast::BinaryOp::Or))
            .or(just(Token::Op(Op::Xor)).to(ast::BinaryOp::Xor))
            .map_with_span(SrcNode::new);
        let logical = comparison.clone()
            .then(op.then(comparison).repeated())
            .foldl(|a, (op, b)| {
                let span = a.span().union(b.span());
                SrcNode::new(ast::Expr::Binary(op, a, b), span)
            })
            .boxed();

        logical
            .map(|expr| expr.into_inner())
    })
        .labelled("expression")
}

pub fn generics_parser() -> impl Parser<ast::Generics> {
    type_ident_parser()
        .map_with_span(SrcNode::new)
        .separated_by(just(Token::Comma))
        .allow_trailing()
        .map(|tys| ast::Generics { tys })
}

pub fn data_parser() -> impl Parser<ast::Data> {
    let variant = type_ident_parser()
        .map_with_span(SrcNode::new)
        .then(type_parser()
            .map_with_span(SrcNode::new)
            .or_not())
        .map(|(name, ty)| {
            let name_span = name.span();
            (name, ty.unwrap_or_else(|| SrcNode::new(ast::Type::Tuple(Vec::new()), name_span)))
        })
        .boxed();

    just(Token::Data)
        .ignore_then(type_ident_parser()
            .map_with_span(SrcNode::new))
        .then(generics_parser())
        .then_ignore(just(Token::Op(Op::Eq)))
        // TODO: Don't use `Result`
        .then(type_parser().map_with_span(SrcNode::new).map(Err)
            .or(variant
                .separated_by(just(Token::Pipe))
                .allow_leading()
                .map(Ok)))
        .map(|((name, generics), variants)| ast::Data {
            generics,
            variants: variants
                .unwrap_or_else(|ty| vec![(name.clone(), ty)]),
            name,
        })
        .boxed()
}

pub fn alias_parser() -> impl Parser<ast::Alias> {
    just(Token::Type)
        .ignore_then(type_ident_parser()
            .map_with_span(SrcNode::new))
        .then(generics_parser())
        .then_ignore(just(Token::Op(Op::Eq)))
        .then(type_parser().map_with_span(SrcNode::new))
        .map(|((name, generics), ty)| ast::Alias {
            name,
            generics,
            ty,
        })
        .boxed()
}

pub fn def_parser() -> impl Parser<ast::Def> {
    just(Token::Def)
        .ignore_then(term_ident_parser()
            .map_with_span(SrcNode::new))
        .then(generics_parser())
        .then(ty_hint_parser())
        .then_ignore(just(Token::Op(Op::Eq)))
        .then(expr_parser()
            .map_with_span(SrcNode::new))
        .map(|(((name, generics), ty_hint), body)| ast::Def {
            generics,
            ty_hint: ty_hint.unwrap_or_else(|| SrcNode::new(ast::Type::Unknown, name.span())),
            name,
            body,
        })
        .boxed()
}

pub fn item_parser() -> impl Parser<ast::Item> {
    def_parser().map(ast::Item::Def)
        .or(data_parser().map(ast::Item::Data))
        .or(alias_parser().map(ast::Item::Alias))
}

pub fn module_parser() -> impl Parser<ast::Module> {
    item_parser()
        .repeated()
        .then_ignore(end())
        .map(|items| ast::Module { items })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chumsky::Parser;

    #[test]
    fn simple() {
        let code = r#"
            4 * 5 + (3 - 2) / foo(3, 4)
        "#;
        let len = code.chars().count();

        let span = |i| Span::new(SrcId::empty(), i..i + 1);

        let tokens = token::lexer()
            .parse(chumsky::Stream::from_iter(
                span(len),
                code.chars().enumerate().map(|(i, c)| (c, span(i))),
            ))
            .unwrap();

        let res = expr_parser()
            .then_ignore(end())
            .parse(chumsky::Stream::from_iter(
                span(len),
                tokens.into_iter(),
            ))
            .unwrap();

        assert_eq!(
            res,
            ast::Expr::Binary(
                SrcNode::new(ast::BinaryOp::Add, Span::empty()),
                SrcNode::new(ast::Expr::Binary(
                    SrcNode::new(ast::BinaryOp::Mul, Span::empty()),
                    SrcNode::new(ast::Expr::Literal(ast::Literal::Nat(4)), Span::empty()),
                    SrcNode::new(ast::Expr::Literal(ast::Literal::Nat(5)), Span::empty()),
                ), Span::empty()),
                SrcNode::new(ast::Expr::Binary(
                    SrcNode::new(ast::BinaryOp::Div, Span::empty()),
                    SrcNode::new(ast::Expr::Binary(
                        SrcNode::new(ast::BinaryOp::Sub, Span::empty()),
                        SrcNode::new(ast::Expr::Literal(ast::Literal::Nat(3)), Span::empty()),
                        SrcNode::new(ast::Expr::Literal(ast::Literal::Nat(2)), Span::empty()),
                    ), Span::empty()),
                    SrcNode::new(ast::Expr::Apply(
                        SrcNode::new(ast::Expr::Apply(
                            SrcNode::new(ast::Expr::Local(ast::Ident::new("foo")), Span::empty()),
                            SrcNode::new(ast::Expr::Literal(ast::Literal::Nat(3)), Span::empty()),
                        ), Span::empty()),
                        SrcNode::new(ast::Expr::Literal(ast::Literal::Nat(4)), Span::empty()),
                    ), Span::empty()),
                ), Span::empty()),
            ),
            "{:#?}", res,
        );
    }
}