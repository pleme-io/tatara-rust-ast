//! The 0.1.11 nodes: a struct, an enum whose variants carry only their own
//! fields, a match over it, a struct literal, field access and signed
//! literals. Rendered through `emit_file`, read back by `syn`, round-tripped
//! through JSON, and compiled by rustc, so a rendering rustc would refuse
//! fails here.

use tatara_rust_ast::{
    Arm, BinOp, Block, Expr, FieldInit, FieldPat, File, Fn, FnParam, FnSig, Generics, Ident,
    IntSuffix, Item, Pattern, RefKind, Stmt, StructField, TypeRef, Variant, Visibility,
};
use tatara_rust_emit::emit_file;

fn id(s: &str) -> Ident {
    Ident::new(s)
}

fn path(segs: &[&str]) -> Expr {
    Expr::Path {
        segments: segs.iter().map(|s| id(s)).collect(),
    }
}

fn i64_ty() -> TypeRef {
    TypeRef::simple("i64")
}

fn field(name: &str) -> StructField {
    StructField {
        vis: Visibility::Private,
        name: id(name),
        ty: i64_ty(),
    }
}

fn bind(name: &str) -> Pattern {
    Pattern::Bind {
        name: id(name),
        by_ref: false,
        mutable: false,
    }
}

fn signed(value: i64) -> Expr {
    Expr::SignedInt {
        value,
        suffix: Some(IntSuffix::I64),
    }
}

fn mul(a: Expr, b: Expr) -> Expr {
    Expr::Binary {
        op: BinOp::Mul,
        lhs: Box::new(a),
        rhs: Box::new(b),
    }
}

fn deref(name: &str) -> Expr {
    Expr::Deref {
        expr: Box::new(path(&[name])),
    }
}

fn shapes() -> File {
    let shape = Item::Enum {
        vis: Visibility::Pub,
        name: id("Shape"),
        derives: vec![id("Clone"), id("Debug")],
        variants: vec![
            Variant {
                name: id("Empty"),
                fields: vec![],
                elems: vec![],
            },
            Variant {
                name: id("Circle"),
                fields: vec![field("r")],
                elems: vec![],
            },
            Variant {
                name: id("Rect"),
                fields: vec![field("w"), field("h")],
                elems: vec![],
            },
            Variant {
                name: id("Boxed"),
                fields: vec![],
                elems: vec![TypeRef::generic("Box", vec![TypeRef::simple("Pair")])],
            },
        ],
    };
    let pair = Item::Struct {
        vis: Visibility::Pub,
        name: id("Pair"),
        derives: vec![id("Clone"), id("Debug")],
        fields: vec![field("a"), field("b")],
    };
    let area = Item::Fn {
        vis: Visibility::Pub,
        item: Fn {
            sig: FnSig {
                name: id("area"),
                generics: Generics::default(),
                params: vec![FnParam {
                    name: "s".into(),
                    ty: TypeRef {
                        ident: id("Shape"),
                        generics: vec![],
                        reference: Some(RefKind::shared()),
                    },
                }],
                return_type: Some(i64_ty()),
            },
            body: Block {
                stmts: vec![Stmt::Tail {
                    expr: Expr::Match {
                        scrutinee: Box::new(path(&["s"])),
                        arms: vec![
                            Arm {
                                pattern: Pattern::Path {
                                    segments: vec![id("Shape"), id("Empty")],
                                },
                                guard: None,
                                body: signed(i64::MIN),
                            },
                            Arm {
                                pattern: Pattern::Struct {
                                    path: vec![id("Shape"), id("Circle")],
                                    fields: vec![FieldPat {
                                        name: id("r"),
                                        pattern: bind("r"),
                                    }],
                                    rest: false,
                                },
                                guard: Some(Expr::Binary {
                                    op: BinOp::Lt,
                                    lhs: Box::new(deref("r")),
                                    rhs: Box::new(signed(-1)),
                                }),
                                body: signed(-1),
                            },
                            Arm {
                                pattern: Pattern::Struct {
                                    path: vec![id("Shape"), id("Circle")],
                                    fields: vec![FieldPat {
                                        name: id("r"),
                                        pattern: bind("r"),
                                    }],
                                    rest: false,
                                },
                                guard: None,
                                body: mul(mul(deref("r"), deref("r")), signed(3)),
                            },
                            Arm {
                                pattern: Pattern::Struct {
                                    path: vec![id("Shape"), id("Rect")],
                                    fields: vec![FieldPat {
                                        name: id("w"),
                                        pattern: bind("w"),
                                    }],
                                    rest: true,
                                },
                                guard: None,
                                body: deref("w"),
                            },
                            Arm {
                                pattern: Pattern::TupleStruct {
                                    path: vec![id("Shape"), id("Boxed")],
                                    elems: vec![Pattern::Wild],
                                },
                                guard: None,
                                body: Expr::Int {
                                    value: 0,
                                    suffix: Some(IntSuffix::I64),
                                },
                            },
                        ],
                    },
                }],
            },
        },
    };
    let sum = Item::Fn {
        vis: Visibility::Pub,
        item: Fn {
            sig: FnSig {
                name: id("sum"),
                generics: Generics::default(),
                params: vec![],
                return_type: Some(i64_ty()),
            },
            body: Block {
                stmts: vec![
                    Stmt::Local {
                        name: id("p"),
                        mutable: false,
                        ty: None,
                        value: Expr::StructLit {
                            path: vec![id("Pair")],
                            fields: vec![
                                FieldInit {
                                    name: id("a"),
                                    value: signed(-5),
                                },
                                FieldInit {
                                    name: id("b"),
                                    value: Expr::Call {
                                        func: vec![id("area")],
                                        args: vec![Expr::Ref {
                                            mutable: false,
                                            expr: Box::new(Expr::StructLit {
                                                path: vec![id("Shape"), id("Rect")],
                                                fields: vec![
                                                    FieldInit {
                                                        name: id("w"),
                                                        value: signed(4),
                                                    },
                                                    FieldInit {
                                                        name: id("h"),
                                                        value: signed(2),
                                                    },
                                                ],
                                            }),
                                        }],
                                    },
                                },
                            ],
                        },
                    },
                    Stmt::Tail {
                        expr: Expr::MethodCall {
                            receiver: Box::new(Expr::Field {
                                base: Box::new(path(&["p"])),
                                field: id("a"),
                            }),
                            method: id("wrapping_add"),
                            args: vec![Expr::Field {
                                base: Box::new(path(&["p"])),
                                field: id("b"),
                            }],
                        },
                    },
                ],
            },
        },
    };
    File {
        attrs: vec![],
        items: vec![shape, pair, area, sum],
    }
}

#[test]
fn records_variants_and_a_match_render_and_reparse() {
    let out = emit_file(&shapes()).expect("renders to a file syn accepts");
    assert!(
        out.contains("#[derive(Clone, Debug)]\npub enum Shape {"),
        "{out}"
    );
    assert!(out.contains("Circle { r: i64 },"), "{out}");
    assert!(out.contains("Rect { w: i64, h: i64 },"), "{out}");
    assert!(out.contains("Boxed(Box<Pair>),"), "{out}");
    assert!(out.contains("Shape::Boxed(_) => 0i64,"), "{out}");
    assert!(out.contains("pub struct Pair {"), "{out}");
    assert!(
        out.contains("Shape::Empty => (-9223372036854775808i64),"),
        "{out}"
    );
    assert!(
        out.contains("Shape::Circle { r: r } if ((*r) < (-1i64)) => (-1i64),"),
        "{out}"
    );
    assert!(out.contains("Shape::Rect { w: w, .. } => (*w),"), "{out}");
    assert!(out.contains("let p = Pair {\n        a: (-5i64),\n        b: area((&Shape::Rect { w: 4i64, h: 2i64 })),\n    };"), "{out}");
    assert!(out.contains("p.a.wrapping_add(p.b)"), "{out}");
}

#[test]
fn the_new_nodes_survive_json() {
    let file = shapes();
    let back: File = serde_json::from_str(&serde_json::to_string(&file).unwrap()).unwrap();
    assert_eq!(file, back);
}

#[test]
fn a_negative_literal_binds_before_a_method_call() {
    let call = Expr::MethodCall {
        receiver: Box::new(signed(-5)),
        method: id("abs"),
        args: vec![],
    };
    let out = emit_file(&Item::Const {
        vis: Visibility::Private,
        name: id("X"),
        ty: i64_ty(),
        value: call,
    })
    .unwrap();
    assert!(out.contains("const X: i64 = (-5i64).abs();"), "{out}");
    let pattern = emit_file(&Item::Const {
        vis: Visibility::Private,
        name: id("Y"),
        ty: i64_ty(),
        value: Expr::Match {
            scrutinee: Box::new(signed(0)),
            arms: vec![
                Arm {
                    pattern: Pattern::Int {
                        value: 0,
                        suffix: Some(IntSuffix::I64),
                    },
                    guard: None,
                    body: signed(0),
                },
                Arm {
                    pattern: Pattern::Int {
                        value: -1,
                        suffix: Some(IntSuffix::I64),
                    },
                    guard: None,
                    body: signed(1),
                },
                Arm {
                    pattern: Pattern::TupleStruct {
                        path: vec![id("Some")],
                        elems: vec![Pattern::Wild],
                    },
                    guard: None,
                    body: signed(2),
                },
                Arm {
                    pattern: Pattern::Bind {
                        name: id("n"),
                        by_ref: true,
                        mutable: true,
                    },
                    guard: None,
                    body: signed(3),
                },
            ],
        },
    })
    .unwrap();
    assert!(pattern.contains("match 0i64 {"), "{pattern}");
    assert!(pattern.contains("0i64 => 0i64,"), "{pattern}");
    assert!(!pattern.contains("-0i64"), "{pattern}");
    assert!(pattern.contains("-1i64 => 1i64,"), "{pattern}");
    assert!(pattern.contains("Some(_) => 2i64,"), "{pattern}");
    assert!(pattern.contains("ref mut n => 3i64,"), "{pattern}");
}

#[test]
fn a_variant_naming_fields_and_elements_is_refused_when_emitted() {
    let both = Item::Enum {
        vis: Visibility::Private,
        name: id("E"),
        derives: vec![],
        variants: vec![Variant {
            name: id("V"),
            fields: vec![field("a")],
            elems: vec![i64_ty()],
        }],
    };
    assert!(emit_file(&both).is_err());
}

#[test]
fn the_rendering_compiles() {
    let Ok(rustc) = std::env::var("RUSTC").or_else(|_| Ok::<_, ()>("rustc".into())) else {
        return;
    };
    let dir = std::env::temp_dir().join(format!("tra-data-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("shapes.rs");
    std::fs::write(&src, emit_file(&shapes()).unwrap()).unwrap();
    let out = std::process::Command::new(rustc)
        .args([
            "--edition",
            "2021",
            "--crate-type",
            "lib",
            "--emit",
            "metadata",
            "-o",
        ])
        .arg(dir.join("libshapes.rmeta"))
        .arg(&src)
        .output();
    let Ok(out) = out else { return };
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}
