#![doc = include_str!("../README.md")]
//! > _Note_: This documentation is auto-generated
//! > from the project's README.md file.
use std::{path::PathBuf, process::Command};

use proc_macro::{TokenStream, TokenTree};

/// Loads a coda from a file, generating Rust data
/// structures and codecs for the coda and exporting
/// them into the module that called this macro.
///
/// Refer to the [crate] docs for more info.
#[proc_macro]
pub fn export_coda(tokens: TokenStream) -> TokenStream {
    let coda_path = parse_token_string(tokens);

    // Locate the workspace root path.
    let workspace_toml = Command::new(env!("CARGO"))
        .arg("locate-project")
        .arg("--workspace")
        .arg("--message-format=plain")
        .output()
        .unwrap()
        .stdout;
    let workspace_toml = String::from_utf8_lossy(&workspace_toml);
    let workspace_root = PathBuf::from(workspace_toml.trim());
    let workspace_root = workspace_root.parent().unwrap();

    // Locate the coda relative to the workspace root.
    let path = workspace_root.join(coda_path);

    // Load and parse the coda (and any codas it uses) from the file.
    let (coda, sources) = codas::parse::load(&path).unwrap();

    // Generate Rust code.
    let mut codegen = vec![];
    #[cfg(feature = "serde")]
    {
        codas::langs::rust::generate_types(&coda, &mut codegen, true).unwrap();
    }
    #[cfg(not(feature = "serde"))]
    {
        codas::langs::rust::generate_types(&coda, &mut codegen, false).unwrap();
    }
    let codegen = String::from_utf8_lossy(&codegen);

    // Prepend the generated code with statements
    // that will trigger a rebuild of the code whenever
    // any source file changes.
    let mut codegen_prefix = String::new();
    for source in sources {
        codegen_prefix += &format!(
            r#"
            const _: &str = include_str!("{}");
        "#,
            source.canonicalize().unwrap().display()
        );
    }
    codegen_prefix += &codegen;

    codegen_prefix.parse().unwrap()
}

/// Parses the first quoted string from `tokens`,
/// returning the string (without quotes).
fn parse_token_string(tokens: TokenStream) -> String {
    // Assume the tokens contain a single string literal.
    let mut coda = match tokens.into_iter().next() {
        Some(TokenTree::Literal(string)) => string.to_string(),
        _ => panic!("rly?"),
    };

    // Strip leading/trailing literal markers.
    if coda.starts_with("r#") {
        coda.remove(0); // r
        coda.remove(0); // #
        coda.remove(coda.len() - 1); // #
    }

    // Strip quotes.
    coda.remove(0);
    coda.remove(coda.len() - 1);

    coda
}
