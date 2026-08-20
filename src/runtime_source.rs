use crate::diagnostic::Diagnostic;
use crate::embedded_vfs::EmbeddedSourceProvider;
use crate::includes::{ModuleLoadOptions, load_compilation_from_provider};
use crate::semantic::{analyze_compilation, ir};
use crate::source::{InMemorySourceProvider, SourceOrigin, Span};

/// Compile one embedded Action! runtime source in an isolated semantic scope.
///
/// Historical runtime files use a pair of bare `MODULE` markers.  The linker
/// gives that source a private named-module identity before sending it through
/// the ordinary frontend, so every backend consumes the same resolved symbols.
pub(crate) fn compile_runtime_unit(
    file_name: &str,
    module_name: &str,
) -> Result<ir::SemProgram, Vec<Diagnostic>> {
    let source = EmbeddedSourceProvider
        .runtime_source(file_name)
        .ok_or_else(|| diagnostic(format!("embedded runtime source `{file_name}` is missing")))?;
    let text = crate::source::decode_source(source.bytes);
    let text = make_internal_named_module(&text, module_name, file_name)?;
    let origin = SourceOrigin::embedded(
        format!("runtime/internal/{file_name}"),
        format!("<runtime:{}>", file_name.to_ascii_uppercase()),
    );
    let provider = InMemorySourceProvider::default().with_source(origin.clone(), text);
    let loaded = load_compilation_from_provider(origin, &provider, &ModuleLoadOptions::default())
        .map_err(|diagnostics| frontend_diagnostics(file_name, diagnostics))?;
    let model = analyze_compilation(&loaded)
        .map_err(|diagnostics| frontend_diagnostics(file_name, diagnostics))?;
    Ok(ir::lower_compilation(&loaded, &model))
}

fn make_internal_named_module(
    source: &str,
    module_name: &str,
    file_name: &str,
) -> Result<String, Vec<Diagnostic>> {
    let mut converted_first_marker = false;
    let mut output = String::with_capacity(source.len() + 32);
    for line in source.lines() {
        let trimmed = line.trim_start();
        if trimmed.to_ascii_uppercase().starts_with("MODULE ;") {
            if !converted_first_marker {
                output.push_str("MODULE ");
                output.push_str(module_name);
                converted_first_marker = true;
            } else {
                output.push_str("ENDMODULE");
            }
        } else if converted_first_marker && trimmed.eq_ignore_ascii_case("MODULE") {
            output.push_str("ENDMODULE");
        } else {
            output.push_str(line);
        }
        output.push('\n');
    }
    if !converted_first_marker {
        return Err(diagnostic(format!(
            "embedded runtime source `{file_name}` has no legacy MODULE marker"
        )));
    }
    Ok(output)
}

fn frontend_diagnostics(file_name: &str, diagnostics: Vec<Diagnostic>) -> Vec<Diagnostic> {
    diagnostics
        .into_iter()
        .map(|diagnostic| {
            Diagnostic::new(
                diagnostic.span,
                format!("embedded {file_name} frontend: {}", diagnostic.message),
            )
        })
        .collect()
}

fn diagnostic(message: impl Into<String>) -> Vec<Diagnostic> {
    vec![Diagnostic::new(Span::new(0, 0), message)]
}
