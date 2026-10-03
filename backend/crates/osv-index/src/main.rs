//! Command line for [`ferrobox_osv_index`].

use std::path::PathBuf;
use std::process::ExitCode;

use ferrobox_osv_index::ConvertError;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("ferrobox-osv-index: {err}");
            ExitCode::from(1)
        }
    }
}

fn run() -> Result<(), ConvertError> {
    let mut dataset = None;
    let mut output = None;
    let mut inputs = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--dataset" => {
                dataset = Some(args.next().ok_or_else(|| {
                    ConvertError::Usage("missing value for --dataset".to_string())
                })?);
            }
            "--output" => {
                output = Some(args.next().ok_or_else(|| {
                    ConvertError::Usage("missing value for --output".to_string())
                })?);
            }
            "--help" | "-h" => {
                println!(
                    "usage: ferrobox-osv-index --dataset YYYY-MM-DD --output index.json.gz <zip-or-json>..."
                );
                return Ok(());
            }
            other if other.starts_with('-') => {
                return Err(ConvertError::Usage(format!("unknown argument {other}")));
            }
            other => inputs.push(PathBuf::from(other)),
        }
    }
    let dataset = dataset.ok_or_else(|| ConvertError::Usage("missing --dataset".to_string()))?;
    let output = output.ok_or_else(|| ConvertError::Usage("missing --output".to_string()))?;
    if inputs.is_empty() {
        return Err(ConvertError::Usage(
            "pass at least one OSV .zip or .json file".to_string(),
        ));
    }
    let stats = ferrobox_osv_index::write_index(&dataset, &inputs, PathBuf::from(output))?;
    println!(
        "files={} withdrawn={} advisories={}",
        stats.files, stats.withdrawn, stats.advisories
    );
    Ok(())
}
