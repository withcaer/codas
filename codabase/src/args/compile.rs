use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};

use codas::{langs, parse, types::Coda};

use super::{open_file_or_stdin, CompileCommand, Lang};

/// Executes `command` locally.
pub fn execute_compile_command(command: CompileCommand) {
    match command.lang {
        Some(lang) => pipe_mode(command.source, lang),
        None => batch_mode(command),
    }
}

/// Compile a single coda and write the output to stdout.
fn pipe_mode(source: Option<PathBuf>, lang: Lang) {
    let mut input = open_file_or_stdin(source).expect("source doesn't exist");
    let mut markdown = String::new();
    input
        .read_to_string(&mut markdown)
        .expect("failed to read source");

    let coda = parse::parse(&markdown).expect("failed to parse coda");
    let mut stdout = std::io::stdout().lock();

    generate(&coda, lang, &mut stdout);
}

/// Compile all codas found in a source directory to all
/// languages, writing output files into the target directory.
fn batch_mode(command: CompileCommand) {
    let source = command
        .source
        .unwrap_or_else(|| std::env::current_dir().unwrap());

    if !source.is_dir() {
        eprintln!(
            "error: --source must be a directory in batch mode (got {})",
            source.display()
        );
        std::process::exit(1);
    }

    let codas = discover_codas(&source);

    if codas.is_empty() {
        eprintln!("no codas found in {}", source.display());
        return;
    }

    let langs = [
        Lang::Rust,
        Lang::Python,
        Lang::Typescript,
        Lang::OpenApi,
        Lang::Duckdb,
        Lang::Sqlite,
    ];

    for lang in langs {
        let lang_dir = command.target.join(lang.dir_name());
        fs::create_dir_all(&lang_dir).expect("failed to create output directory");

        for (path, coda) in &codas {
            let file_name = lang.file_name(&coda.local_name);
            let out_path = lang_dir.join(&file_name);
            let mut file = fs::File::create(&out_path).expect("failed to create output file");

            generate(coda, lang, &mut file);
            eprintln!("  {} -> {}", path.display(), out_path.display());
        }
    }

    eprintln!(
        "compiled {} coda(s) to {} language(s)",
        codas.len(),
        langs.len()
    );
}

/// Recursively discover and parse all coda markdown files
/// under `dir`, returning the successfully parsed codas
/// alongside their source paths.
fn discover_codas(dir: &Path) -> Vec<(PathBuf, Coda)> {
    let mut codas = Vec::new();
    collect_md_files(dir, &mut codas);
    codas.sort_by(|(a, _), (b, _)| a.cmp(b));
    codas
}

/// Recursively collects `.md` files from `dir`, attempting
/// to parse each as a coda.
fn collect_md_files(dir: &Path, codas: &mut Vec<(PathBuf, Coda)>) {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();

        if path.is_dir() {
            collect_md_files(&path, codas);
        } else if path.extension().is_some_and(|ext| ext == "md") {
            let markdown = match fs::read_to_string(&path) {
                Ok(m) => m,
                Err(_) => continue,
            };

            if let Ok(coda) = parse::parse(&markdown) {
                codas.push((path, coda));
            }
        }
    }
}

/// Generate code for `coda` in the given `lang`, writing to `out`.
fn generate(coda: &Coda, lang: Lang, out: &mut impl std::io::Write) {
    match lang {
        Lang::Rust => langs::rust::generate_types(coda, out, true),
        Lang::Python => langs::python::generate_types(coda, out),
        Lang::Typescript => langs::typescript::generate_types(coda, out),
        Lang::OpenApi => langs::open_api::generate_spec(coda, out),
        Lang::Duckdb => langs::duckdb::generate_types(coda, out),
        Lang::Sqlite => langs::sqlite::generate_types(coda, out),
    }
    .expect("failed to write output");
}

impl Lang {
    /// Subdirectory name for this language's output.
    fn dir_name(self) -> &'static str {
        match self {
            Lang::Rust => "rust",
            Lang::Python => "python",
            Lang::Typescript => "typescript",
            Lang::OpenApi => "open-api",
            Lang::Duckdb => "duckdb",
            Lang::Sqlite => "sqlite",
        }
    }

    /// Output file name for a coda with the given local name.
    fn file_name(self, local_name: &str) -> String {
        let snake = to_snake_case(local_name);
        match self {
            Lang::Rust => format!("{snake}.rs"),
            Lang::Python => format!("{snake}.py"),
            Lang::Typescript => format!("{snake}.ts"),
            Lang::OpenApi => format!("{snake}.yaml"),
            Lang::Duckdb => format!("{snake}.sql"),
            Lang::Sqlite => format!("{snake}.sql"),
        }
    }
}

/// Converts a CamelCase name to snake_case.
fn to_snake_case(s: &str) -> String {
    let mut result = String::with_capacity(s.len() + 4);
    for (i, c) in s.chars().enumerate() {
        if c.is_uppercase() && i > 0 {
            result.push('_');
        }
        result.push(c.to_lowercase().next().unwrap());
    }
    result
}
