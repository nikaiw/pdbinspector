use std::process::Command;

/// Test fixture from getsentry/pdb repository
const TEST_PDB: &str = "tests/fixtures/foo.pdb";

/// Run pdbinspector with given arguments and return stdout
fn run_pdbinspector(args: &[&str]) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_pdbinspector"))
        .args(args)
        .output()
        .expect("Failed to execute pdbinspector");

    assert!(
        output.status.success(),
        "pdbinspector failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    String::from_utf8(output.stdout).expect("Invalid UTF-8 output")
}

#[test]
fn test_parse_pdb_json() {
    let output = run_pdbinspector(&["report", TEST_PDB, "--format", "json"]);
    let report: serde_json::Value = serde_json::from_str(&output).expect("Invalid JSON output");

    // Verify status
    assert_eq!(report["status"], "ok");
    assert_eq!(report["file_path"], TEST_PDB);

    // Verify type counts
    assert_eq!(report["types"]["classes"]["count"], 418);
    assert_eq!(report["types"]["structs"]["count"], 585);
    assert_eq!(report["types"]["unions"]["count"], 40);
    assert_eq!(report["types"]["enumerations"]["count"], 142);
    assert_eq!(report["types"]["procedures"]["count"], 794);
    assert_eq!(report["types"]["arrays"]["count"], 218);
    assert_eq!(report["types"]["pointers"]["count"], 1625);

    // Verify symbol counts
    assert_eq!(report["symbols"]["functions"]["count"], 2662);
    assert_eq!(report["symbols"]["global_data"]["count"], 430);
    assert_eq!(report["symbols"]["public_functions"]["count"], 2256);
    assert_eq!(report["symbols"]["public_data"]["count"], 1150);
    assert_eq!(report["symbols"]["constants"]["count"], 554);

    // Verify summary totals
    assert_eq!(report["summary"]["total_types"], 8406);
    assert_eq!(report["summary"]["total_symbols"], 10842);

    // Verify summary flags
    assert_eq!(report["summary"]["has_functions"], true);
    assert_eq!(report["summary"]["has_structs"], true);
    assert_eq!(report["summary"]["has_classes"], true);
    assert_eq!(report["summary"]["has_enums"], true);
    assert_eq!(report["summary"]["has_unions"], true);
    assert_eq!(report["summary"]["has_global_data"], true);
    assert_eq!(report["summary"]["has_public_functions"], true);
    assert_eq!(report["summary"]["has_public_data"], true);
}

#[test]
fn test_text_output() {
    let output = run_pdbinspector(&["report", TEST_PDB, "--format", "text"]);

    // Verify key sections are present
    assert!(output.contains("=== PDB Report:"));
    assert!(output.contains("Summary:"));
    assert!(output.contains("Type Statistics:"));
    assert!(output.contains("Symbol Statistics:"));

    // Verify counts appear in output
    assert!(output.contains("Classes:"));
    assert!(output.contains("Structs:"));
    assert!(output.contains("Functions:"));
}

#[test]
fn test_grep_output_with_detailed() {
    let output = run_pdbinspector(&["report", TEST_PDB, "--format", "grep", "--detailed"]);

    // Verify tab-separated format
    let lines: Vec<&str> = output.lines().collect();
    assert!(!lines.is_empty());

    // Check that we have entries of various types
    let has_class = lines.iter().any(|l| l.starts_with("class\t"));
    let has_struct = lines.iter().any(|l| l.starts_with("struct\t"));
    let has_function = lines.iter().any(|l| l.starts_with("function\t"));
    let has_public_function = lines.iter().any(|l| l.starts_with("public_function\t"));

    assert!(has_class, "Expected class entries in grep output");
    assert!(has_struct, "Expected struct entries in grep output");
    assert!(has_function, "Expected function entries in grep output");
    assert!(
        has_public_function,
        "Expected public_function entries in grep output"
    );
}

#[test]
fn test_detailed_flag() {
    let output = run_pdbinspector(&["report", TEST_PDB, "--format", "json", "--detailed"]);
    let report: serde_json::Value = serde_json::from_str(&output).expect("Invalid JSON output");

    // With --detailed, we should have type_details and symbol_details
    assert!(report["type_details"].is_object());
    assert!(report["symbol_details"].is_object());

    // Verify detailed lists have content
    assert!(report["type_details"]["classes"].as_array().unwrap().len() > 0);
    assert!(report["type_details"]["structs"].as_array().unwrap().len() > 0);
    assert!(
        report["symbol_details"]["functions"]
            .as_array()
            .unwrap()
            .len()
            > 0
    );
}
