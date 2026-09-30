//! The 0.1.8 nodes, rendered through `emit_file` and read back as Rust: a
//! `no_std` file with typed control flow, arithmetic and a const. The output
//! is re-parsed by `syn` inside `emit_file`, so a malformed rendering fails
//! here, and the strings below pin what a reader sees.

use tatara_rust_ast::{
    BinOp, Block, Expr, File, Fn, FnParam, FnSig, Generics, Ident, InnerAttr, IntSuffix, Item,
    Stmt, TypeRef, Visibility,
};
use tatara_rust_emit::emit_file;

fn id(s: &str) -> Ident {
    Ident::new(s)
}

fn path(s: &str) -> Expr {
    Expr::Path {
        segments: vec![id(s)],
    }
}

#[test]
fn a_no_std_file_with_a_loop_renders_and_reparses() {
    let count = Item::Fn {
        vis: Visibility::Pub,
        item: Fn {
            sig: FnSig {
                name: id("count"),
                generics: Generics::default(),
                params: vec![FnParam {
                    name: "n".into(),
                    ty: TypeRef::simple("u32"),
                }],
                return_type: Some(TypeRef::simple("u32")),
            },
            body: Block {
                stmts: vec![
                    Stmt::Local {
                        name: id("i"),
                        mutable: true,
                        ty: Some(TypeRef::simple("u32")),
                        value: Expr::Int {
                            value: 0,
                            suffix: Some(IntSuffix::U32),
                        },
                    },
                    Stmt::Semi {
                        expr: Expr::Loop {
                            body: Block {
                                stmts: vec![
                                    Stmt::Semi {
                                        expr: Expr::If {
                                            cond: Box::new(Expr::Binary {
                                                op: BinOp::Ge,
                                                lhs: Box::new(path("i")),
                                                rhs: Box::new(path("n")),
                                            }),
                                            then_branch: Block {
                                                stmts: vec![Stmt::Semi { expr: Expr::Break }],
                                            },
                                            else_branch: None,
                                        },
                                    },
                                    Stmt::Semi {
                                        expr: Expr::Assign {
                                            target: id("i"),
                                            value: Box::new(Expr::Call {
                                                func: vec![id("step")],
                                                args: vec![path("i")],
                                            }),
                                        },
                                    },
                                ],
                            },
                        },
                    },
                    Stmt::Tail { expr: path("i") },
                ],
            },
        },
    };
    let file = File {
        attrs: vec![
            InnerAttr {
                path: vec![id("no_std")],
                args: vec![],
            },
            InnerAttr {
                path: vec![id("forbid")],
                args: vec![id("unsafe_code")],
            },
        ],
        items: vec![
            Item::Const {
                vis: Visibility::Private,
                name: id("UART"),
                ty: TypeRef::simple("usize"),
                value: Expr::Int {
                    value: 0x1000_0000,
                    suffix: None,
                },
            },
            count,
        ],
    };
    let out = emit_file(&file).expect("renders to a file syn accepts");
    assert!(out.starts_with("#![no_std]\n#![forbid(unsafe_code)]\n"), "{out}");
    assert!(out.contains("const UART: usize = 268435456;"), "{out}");
    assert!(out.contains("pub fn count(n: u32) -> u32 {"), "{out}");
    assert!(out.contains("let mut i: u32 = 0u32;"), "{out}");
    assert!(out.contains("if (i >= n) {"), "{out}");
    assert!(out.contains("i = step(i);"), "{out}");

    // The same tree survives JSON, which is how a compiler outside Rust hands
    // it over.
    let json = serde_json::to_string(&file).unwrap();
    let back: File = serde_json::from_str(&json).unwrap();
    assert_eq!(file, back);
}

#[test]
fn an_unknown_node_kind_is_refused_at_the_border() {
    let bad = r#"{"items":[{"kind":"fn","item":{"sig":{"name":"f"},"body":{"stmts":[{"kind":"tail","expr":{"kind":"goto","label":"x"}}]}}}]}"#;
    assert!(serde_json::from_str::<File>(bad).is_err());
}
