use std::{io::Write, num::NonZeroU32, path::PathBuf, process::ExitCode};

use cherenkov_oracle::metrics;
use clap::{Args as ClapArgs, Parser, Subcommand};
use serde::Serialize;

use super::{Error, decode, score};

#[derive(Parser)]
#[command(about = "Color-managed FLIP comparison of Apple E2E PNG screenshots")]
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Validate PNG decoding, not whether the rendered contents are correct.
    Check { image: PathBuf },
    /// Compare full frames and write a perceptual error heatmap.
    Compare(Compare),
}

#[derive(ClapArgs)]
struct Compare {
    reference: PathBuf,
    actual: PathBuf,
    heatmap: PathBuf,
    /// Reviewed parity limits; omit for the oracle's default limits.
    #[arg(long, requires = "os_major")]
    tolerances: Option<PathBuf>,
    #[arg(long, value_parser = ["ios", "macos"])]
    platform: String,
    /// OS major; required when selecting reviewed, versioned limits.
    #[arg(long)]
    os_major: Option<NonZeroU32>,
    #[arg(long)]
    example: String,
    /// Record a comparison without treating its score as acceptance.
    #[arg(long)]
    record: bool,
}

#[derive(Serialize)]
struct Decoded {
    image: PathBuf,
    width: u32,
    height: u32,
    working_space: &'static str,
    visual_review: &'static str,
}

#[derive(Serialize)]
struct Comparison {
    platform: String,
    os_major: Option<u32>,
    example: String,
    reference: PathBuf,
    actual: PathBuf,
    heatmap: PathBuf,
    width: u32,
    height: u32,
    working_space: &'static str,
    #[serde(flatten)]
    score: score::Score,
    limits: score::Limits,
    within_tolerance: bool,
    mode: &'static str,
    visual_review: &'static str,
}

fn emit(value: &impl Serialize) -> Result<(), Error> {
    let json = serde_json::to_vec(value)?;
    let mut stdout = std::io::stdout().lock();
    stdout.write_all(&json)?;
    stdout.write_all(b"\n")?;
    Ok(())
}

pub fn run() -> Result<ExitCode, Error> {
    match Args::parse().command {
        Command::Check { image } => {
            let decoded = decode::load(&image)?;
            emit(&Decoded {
                image,
                width: decoded.width,
                height: decoded.height,
                working_space: "premultiplied-extended-linear-display-p3",
                visual_review: "required",
            })?;
            Ok(ExitCode::SUCCESS)
        }
        Command::Compare(args) => compare(args),
    }
}

fn compare(args: Compare) -> Result<ExitCode, Error> {
    let limits = if let Some(path) = &args.tolerances {
        score::load_limits(
            path,
            &args.platform,
            args.os_major
                .expect("Clap requires --os-major with --tolerances")
                .get(),
            &args.example,
        )?
    } else {
        score::Limits::default()
    };
    let reference = decode::load(&args.reference)?;
    let actual = decode::load(&args.actual)?;
    let (score, heatmap) = score::compare(&reference, &actual)?;
    metrics::write_heatmap(&heatmap, reference.width, reference.height, &args.heatmap)?;
    let within_tolerance = limits.accepts(&score);
    emit(&Comparison {
        platform: args.platform,
        os_major: args.os_major.map(NonZeroU32::get),
        example: args.example,
        reference: args.reference,
        actual: args.actual,
        heatmap: args.heatmap,
        width: reference.width,
        height: reference.height,
        working_space: "premultiplied-extended-linear-display-p3",
        score,
        limits,
        within_tolerance,
        mode: if args.record { "record" } else { "gate" },
        visual_review: "required",
    })?;
    Ok(if within_tolerance || args.record {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::Args;

    fn compare_args() -> Vec<&'static str> {
        vec![
            "compare-screenshots",
            "compare",
            "reference.png",
            "actual.png",
            "heatmap.png",
            "--platform",
            "ios",
            "--example",
            "reminders",
        ]
    }

    #[test]
    fn default_limits_do_not_require_unknown_os_metadata() {
        assert!(Args::try_parse_from(compare_args()).is_ok());
    }

    #[test]
    fn reviewed_limits_require_an_explicit_os_version() {
        let mut args = compare_args();
        args.extend(["--tolerances", "limits.json"]);
        assert!(Args::try_parse_from(args.iter().copied()).is_err());
        args.extend(["--os-major", "27"]);
        assert!(Args::try_parse_from(args).is_ok());
    }
}
