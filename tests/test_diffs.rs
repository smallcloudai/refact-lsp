use std::process::Command;

#[test]
fn test_python_script() {
    let output = Command::new("python")
        .arg("tests/test_diff_handlers.py")
        .output()
        .expect("Failed to execute test_diff_handlers.py");

    assert!(output.status.success(), "Python script failed: {:?}", String::from_utf8_lossy(&output.stderr));

    println!("test_diff_handlers.py output:");
    println!("{}", String::from_utf8_lossy(&output.stdout));
}
