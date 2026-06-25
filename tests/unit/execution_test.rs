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
    let result = Engine::execute(lang, source.to_string(), "Hello 🌎".to_string(), Limits::default())
        .await
        .expect("Execution failed");

    assert!(matches!(result.status, ExecutionStatus::Accepted));
    assert_eq!(result.stdout, "🚀 Hello 🌎\n");
    assert_eq!(result.exit_code, 0);
}

#[tokio::test]
async fn test_c_stdin_passing() {
    let lang = Arc::new(C);
    let source = r#"
#include <stdio.h>
int main() {
    char buf[100];
    if (fgets(buf, sizeof(buf), stdin)) {
        printf("stdin: %s", buf);
    }
    return 0;
}
"#;
    let result = Engine::execute(lang, source.to_string(), "c stdin data".to_string(), Limits::default())
        .await
        .expect("Execution failed");

    assert!(matches!(result.status, ExecutionStatus::Accepted));
    assert_eq!(result.stdout, "stdin: c stdin data");
    assert_eq!(result.exit_code, 0);
}

#[tokio::test]
async fn test_cpp_stdin_passing() {
    let lang = Arc::new(Cpp);
    let source = r#"
#include <iostream>
#include <string>
int main() {
    std::string s;
    if (std::getline(std::cin, s)) {
        std::cout << "stdin: " << s << std::endl;
    }
    return 0;
}
"#;
    let result = Engine::execute(lang, source.to_string(), "cpp stdin data".to_string(), Limits::default())
        .await
        .expect("Execution failed");

    assert!(matches!(result.status, ExecutionStatus::Accepted));
    assert_eq!(result.stdout, "stdin: cpp stdin data\n");
    assert_eq!(result.exit_code, 0);
}

#[tokio::test]
async fn test_javascript_stdin_passing() {
    let lang = Arc::new(JavaScript);
    let source = r#"
const fs = require('fs');
const data = fs.readFileSync(0, 'utf-8');
console.log("stdin: " + data);
"#;
    let result = Engine::execute(lang, source.to_string(), "js stdin data".to_string(), Limits::default())
        .await
        .expect("Execution failed");

    assert!(matches!(result.status, ExecutionStatus::Accepted));
    assert_eq!(result.stdout, "stdin: js stdin data\n");
    assert_eq!(result.exit_code, 0);
}

#[tokio::test]
async fn test_stderr_capture() {
    let limits = Limits::default();
    
    // C
    let c_res = Engine::execute(
        Arc::new(C),
        r#"
#include <stdio.h>
int main() {
    fprintf(stderr, "c error output\n");
    return 0;
}
"#.to_string(),
        String::new(),
        limits.clone(),
    ).await.unwrap();
    assert_eq!(c_res.stderr, "c error output\n");

    // C++
    let cpp_res = Engine::execute(
        Arc::new(Cpp),
        r#"
#include <iostream>
int main() {
    std::cerr << "cpp error output" << std::endl;
    return 0;
}
"#.to_string(),
        String::new(),
        limits.clone(),
    ).await.unwrap();
    assert_eq!(cpp_res.stderr, "cpp error output\n");

    // Python
    let py_res = Engine::execute(
        Arc::new(Python),
        r#"
import sys
sys.stderr.write("python error output\n")
"#.to_string(),
        String::new(),
        limits.clone(),
    ).await.unwrap();
    assert_eq!(py_res.stderr, "python error output\n");

    // JavaScript
    let js_res = Engine::execute(
        Arc::new(JavaScript),
        r#"
console.error("js error output");
"#.to_string(),
        String::new(),
        limits.clone(),
    ).await.unwrap();
    assert_eq!(js_res.stderr, "js error output\n");
}

#[tokio::test]
async fn test_non_zero_exit_codes() {
    let limits = Limits::default();

    // C
    let c_res = Engine::execute(
        Arc::new(C),
        r#"
#include <stdlib.h>
int main() {
    exit(42);
}
"#.to_string(),
        String::new(),
        limits.clone(),
    ).await.unwrap();
    assert_eq!(c_res.status, ExecutionStatus::RuntimeError);
    assert_eq!(c_res.exit_code, 42);

    // C++
    let cpp_res = Engine::execute(
        Arc::new(Cpp),
        r#"
#include <stdlib.h>
int main() {
    exit(42);
}
"#.to_string(),
        String::new(),
        limits.clone(),
    ).await.unwrap();
    assert_eq!(cpp_res.status, ExecutionStatus::RuntimeError);
    assert_eq!(cpp_res.exit_code, 42);

    // Python
    let py_res = Engine::execute(
        Arc::new(Python),
        r#"
import sys
sys.exit(42)
"#.to_string(),
        String::new(),
        limits.clone(),
    ).await.unwrap();
    assert_eq!(py_res.status, ExecutionStatus::RuntimeError);
    assert_eq!(py_res.exit_code, 42);

    // JavaScript
    let js_res = Engine::execute(
        Arc::new(JavaScript),
        r#"
process.exit(42);
"#.to_string(),
        String::new(),
        limits.clone(),
    ).await.unwrap();
    assert_eq!(js_res.status, ExecutionStatus::RuntimeError);
    assert_eq!(js_res.exit_code, 42);
}

#[tokio::test]
async fn test_empty_source_cpp_js() {
    let limits = Limits::default();

    // C++ Compilation Error
    let cpp_res = Engine::execute(
        Arc::new(Cpp),
        String::new(),
        String::new(),
        limits.clone(),
    ).await.unwrap();
    assert_eq!(cpp_res.status, ExecutionStatus::CompilationError);

    // JS Accepted but empty
    let js_res = Engine::execute(
        Arc::new(JavaScript),
        String::new(),
        String::new(),
        limits.clone(),
    ).await.unwrap();
    assert_eq!(js_res.status, ExecutionStatus::Accepted);
    assert!(js_res.stdout.is_empty());
    assert_eq!(js_res.exit_code, 0);
}

#[tokio::test]
async fn test_large_stdin() {
    // Generate 1.5MB of data
    let large_data = "a".repeat(1500000);
    let lang = Arc::new(Python);
    let source = r#"
import sys
data = sys.stdin.read()
print(len(data))
"#;
    let result = Engine::execute(lang, source.to_string(), large_data, Limits::default())
        .await
        .expect("Execution failed");

    println!("DEBUG LARGE STDIN RESULT: status={:?}, stdout={:?}, stderr={:?}, exit_code={:?}, memory_kb={:?}", result.status, result.stdout, result.stderr, result.exit_code, result.memory_kb);
    assert!(matches!(result.status, ExecutionStatus::Accepted));
    assert_eq!(result.stdout.trim(), "1500000");
    assert_eq!(result.exit_code, 0);
}

#[tokio::test]
async fn test_unicode_support_additional() {
    let limits = Limits::default();
    
    // JS
    let js_res = Engine::execute(
        Arc::new(JavaScript),
        r#"
const fs = require('fs');
const data = fs.readFileSync(0, 'utf-8').trim();
console.log("🌟 " + data);
"#.to_string(),
        "Hello 🌎 Unicode 🚀".to_string(),
        limits.clone(),
    ).await.unwrap();
    assert_eq!(js_res.status, ExecutionStatus::Accepted);
    assert_eq!(js_res.stdout.trim(), "🌟 Hello 🌎 Unicode 🚀");

    // C++
    let cpp_res = Engine::execute(
        Arc::new(Cpp),
        r#"
#include <iostream>
#include <string>
int main() {
    std::string s;
    if (std::getline(std::cin, s)) {
        std::cout << "🌟 " << s << std::endl;
    }
    return 0;
}
"#.to_string(),
        "Hello 🌎 Unicode 🚀".to_string(),
        limits.clone(),
    ).await.unwrap();
    assert_eq!(cpp_res.status, ExecutionStatus::Accepted);
    assert_eq!(cpp_res.stdout.trim(), "🌟 Hello 🌎 Unicode 🚀");
}

#[tokio::test]
async fn test_cpp_compilation_error() {
    let lang = Arc::new(Cpp);
    let source = r#"
#include <iostream>
int main() {
    std::cout << "hello"
}
"#;
    let result = Engine::execute(lang, source.to_string(), String::new(), Limits::default())
        .await
        .expect("Execution failed");

    assert!(matches!(result.status, ExecutionStatus::CompilationError));
    assert!(!result.compile_output.is_empty());
}

