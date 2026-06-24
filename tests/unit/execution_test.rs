use std::sync::Arc;
use otter::execution::engine::Engine;
use otter::execution::languages::c::C;
use otter::execution::languages::cpp::Cpp;
use otter::execution::languages::python::Python;
use otter::execution::languages::javascript::JavaScript;
use otter::execution::limits::Limits;
use otter::execution::result::ExecutionStatus;

#[tokio::test]
async fn test_c_hello_world() {
    let lang = Arc::new(C);
    let source = r#"
#include <stdio.h>
int main() {
    printf("hello from c\n");
    return 0;
}
"#;
    let result = Engine::execute(lang, source.to_string(), String::new(), Limits::default())
        .await
        .expect("Execution failed");

    assert!(matches!(result.status, ExecutionStatus::Accepted));
    assert_eq!(result.stdout, "hello from c\n");
    assert_eq!(result.exit_code, 0);
}

#[tokio::test]
async fn test_cpp_hello_world() {
    let lang = Arc::new(Cpp);
    let source = r#"
#include <iostream>
int main() {
    std::cout << "hello from cpp" << std::endl;
    return 0;
}
"#;
    let result = Engine::execute(lang, source.to_string(), String::new(), Limits::default())
        .await
        .expect("Execution failed");

    assert!(matches!(result.status, ExecutionStatus::Accepted));
    assert_eq!(result.stdout, "hello from cpp\n");
    assert_eq!(result.exit_code, 0);
}

#[tokio::test]
async fn test_python_hello_world() {
    let lang = Arc::new(Python);
    let source = "print(\"hello from python\")";
    let result = Engine::execute(lang, source.to_string(), String::new(), Limits::default())
        .await
        .expect("Execution failed");

    assert!(matches!(result.status, ExecutionStatus::Accepted));
    assert_eq!(result.stdout, "hello from python\n");
    assert_eq!(result.exit_code, 0);
}

#[tokio::test]
async fn test_javascript_hello_world() {
    let lang = Arc::new(JavaScript);
    let source = "console.log(\"hello from javascript\");";
    let result = Engine::execute(lang, source.to_string(), String::new(), Limits::default())
        .await
        .expect("Execution failed");

    assert!(matches!(result.status, ExecutionStatus::Accepted));
    assert_eq!(result.stdout, "hello from javascript\n");
    assert_eq!(result.exit_code, 0);
}

#[tokio::test]
async fn test_stdin_passing() {
    let lang = Arc::new(Python);
    let source = r#"
import sys
data = sys.stdin.read()
print(f"stdin: {data}")
"#;
    let result = Engine::execute(lang, source.to_string(), "foo bar".to_string(), Limits::default())
        .await
        .expect("Execution failed");

    assert!(matches!(result.status, ExecutionStatus::Accepted));
    assert_eq!(result.stdout, "stdin: foo bar\n");
    assert_eq!(result.exit_code, 0);
}

#[tokio::test]
async fn test_compilation_error() {
    let lang = Arc::new(C);
    let source = r#"
#include <stdio.h>
int main() {
    printf("hello"
}
"#;
    let result = Engine::execute(lang, source.to_string(), String::new(), Limits::default())
        .await
        .expect("Execution failed");

    assert!(matches!(result.status, ExecutionStatus::CompilationError));
    assert!(!result.compile_output.is_empty());
    assert!(result.compile_output.contains("error") || result.compile_output.contains("expected"));
}

#[tokio::test]
async fn test_empty_source_code() {
    // C fails to compile due to missing main
    let lang_c = Arc::new(C);
    let result_c = Engine::execute(lang_c, String::new(), String::new(), Limits::default())
        .await
        .expect("Execution failed");
    assert!(matches!(result_c.status, ExecutionStatus::CompilationError));

    // Python succeeds but does nothing
    let lang_py = Arc::new(Python);
    let result_py = Engine::execute(lang_py, String::new(), String::new(), Limits::default())
        .await
        .expect("Execution failed");
    assert!(matches!(result_py.status, ExecutionStatus::Accepted));
    assert!(result_py.stdout.is_empty());
    assert_eq!(result_py.exit_code, 0);
}

#[tokio::test]
async fn test_unicode_support() {
    let lang = Arc::new(Python);
    let source = r#"
import sys
data = sys.stdin.read()
print(f"🚀 {data}")
"#;
    let result = Engine::execute(lang, source.to_string(), "こんにちは".to_string(), Limits::default())
        .await
        .expect("Execution failed");

    assert!(matches!(result.status, ExecutionStatus::Accepted));
    assert_eq!(result.stdout, "🚀 こんにちは\n");
    assert_eq!(result.exit_code, 0);
}
