use std::fs;
use cli::run_source;

#[test]
fn test_program_arithmetic_comparison() {
    let source = fs::read_to_string("../../tests/programs/arithmetic_comparison.masm")
        .or_else(|_| fs::read_to_string("tests/programs/arithmetic_comparison.masm"))
        .expect("Failed to read arithmetic_comparison.masm");

    let (_rt, task) = run_source(&source).expect("Failed to run arithmetic_comparison.masm");

    assert_eq!(task.registers[2].as_int(), Some(130));
    assert_eq!(task.registers[3].as_int(), Some(70));
    assert_eq!(task.registers[4].as_int(), Some(3000));
    assert_eq!(task.registers[5].as_int(), Some(3));
    assert_eq!(task.registers[6].as_int(), Some(10));
    assert_eq!(task.registers[7].as_int(), Some(-100));

    assert_eq!(task.registers[10].as_int(), Some(3));
    assert_eq!(task.registers[11].as_int(), Some(63));
    assert_eq!(task.registers[12].as_int(), Some(60));
    assert_eq!(task.registers[15].as_int(), Some(60));
    assert_eq!(task.registers[16].as_int(), Some(3));

    assert_eq!(task.registers[17].as_bool(), Some(false));
    assert_eq!(task.registers[18].as_bool(), Some(true));
    assert_eq!(task.registers[19].as_bool(), Some(true));
    assert_eq!(task.registers[20].as_bool(), Some(true));
    assert_eq!(task.registers[21].as_bool(), Some(true));
    assert_eq!(task.registers[22].as_bool(), Some(false));

    assert_eq!(task.registers[23].as_bool(), Some(true));
    assert_eq!(task.registers[24].as_bool(), Some(true));
    assert_eq!(task.registers[25].as_bool(), Some(false));
    assert_eq!(task.registers[26].as_bool(), Some(true));

    assert_eq!(task.registers[27].as_int(), Some(130));
    let float_res = task.registers[30].as_float().unwrap();
    assert!((float_res - (3.5 + 2.5)).abs() < 1e-5);
}

#[test]
fn test_program_control_flow() {
    let source = fs::read_to_string("../../tests/programs/control_flow.masm")
        .or_else(|_| fs::read_to_string("tests/programs/control_flow.masm"))
        .expect("Failed to read control_flow.masm");

    let (_rt, task) = run_source(&source).expect("Failed to run control_flow.masm");

    assert_eq!(task.registers[2].as_int(), Some(110));
    assert_eq!(task.registers[0].as_int(), Some(21));
}

#[test]
fn test_program_function_calls() {
    let source = fs::read_to_string("../../tests/programs/function_calls.masm")
        .or_else(|_| fs::read_to_string("tests/programs/function_calls.masm"))
        .expect("Failed to read function_calls.masm");

    let (_rt, task) = run_source(&source).expect("Failed to run function_calls.masm");

    assert_eq!(task.registers[2].as_int(), Some(25));

    assert_eq!(task.registers[4].as_int(), Some(55));
}

#[test]
fn test_program_objects() {
    let source = fs::read_to_string("../../tests/programs/objects.masm")
        .or_else(|_| fs::read_to_string("tests/programs/objects.masm"))
        .expect("Failed to read objects.masm");

    let (_rt, task) = run_source(&source).expect("Failed to run objects.masm");

    assert_eq!(task.registers[8].as_int(), Some(60));
    assert_eq!(task.registers[9].as_int(), Some(101));

    assert_eq!(task.registers[16].as_int(), Some(600));
}
