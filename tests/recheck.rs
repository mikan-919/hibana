use hibana::parser::{Expr, NodeId, Stmt};

#[test]
fn identified_conditionals_and_lambdas_use_the_expected_type() {
    let source = "fn choose(flag: bool) -> int[n -> n > 0] { if flag { 1 } else { 2 } }\nfn main() { let add: (int) -> int = fn(n) { n + 1 }\n put(add(choose(true))) }";
    let mut program = hibana::parser::parse(source).unwrap();
    let conditional_id = NodeId::default();
    let lambda_id = NodeId::default();
    let choose = program
        .fns
        .iter_mut()
        .find(|function| function.name == "choose")
        .unwrap();
    let conditional = choose.body.tail.take().unwrap();
    choose.body.tail = Some(Box::new(Expr::Id(conditional_id, conditional)));
    let main = program
        .fns
        .iter_mut()
        .find(|function| function.name == "main")
        .unwrap();
    let Stmt::Let { init, .. } = &mut main.body.stmts[0] else {
        panic!()
    };
    let lambda = std::mem::replace(init, Expr::Int(0));
    *init = Expr::Id(lambda_id, Box::new(lambda));

    hibana::typeck::check(&mut program).unwrap();
    let choose = program
        .fns
        .iter()
        .find(|function| function.name == "choose")
        .unwrap();
    assert!(matches!(choose.body.tail.as_deref(), Some(Expr::Id(id, _)) if *id == conditional_id));
    let main = program
        .fns
        .iter()
        .find(|function| function.name == "main")
        .unwrap();
    assert!(
        matches!(&main.body.stmts[0], Stmt::Let { init: Expr::Id(id, _), .. } if *id == lambda_id)
    );
    let mut output = Vec::new();
    hibana::eval::run(&program, &mut output).unwrap();
    assert_eq!(output, b"2\n");
}
