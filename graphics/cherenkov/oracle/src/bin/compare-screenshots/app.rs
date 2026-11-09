use std::{io::Write, num::NonZeroU32, path::PathBuf, process::ExitCode};

use cherenkov_oracle::metrics;
use clap::{Args as ClapArgs, Parser, Subcommand, ValueEnum};
use serde::Serialize;

use super::{
    Error, decode,
    score::{self, Limits, RouteLimits, TwinLimits},
};

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
    /// Acceptance rule: `route` for two routes to the same pixels (frame
    /// settling, stored baselines), `twin` for a `SwiftUI` twin.
    #[arg(long, value_enum, default_value_t = LimitSet::Route)]
    limits: LimitSet,
    /// Pixels per point of both captures, the backing scale of the display
    /// they were taken on. Required by `twin`, whose block is sized in
    /// points.
    #[arg(long, required_if_eq("limits", "twin"))]
    scale: Option<NonZeroU32>,
    #[arg(long, value_parser = ["ios", "macos"])]
    platform: String,
    /// OS major the capture was taken on, recorded in the report.
    #[arg(long)]
    os_major: Option<NonZeroU32>,
    #[arg(long)]
    example: String,
    /// Record a comparison without treating its score as acceptance.
    #[arg(long)]
    record: bool,
}

#[derive(Clone, Copy, ValueEnum)]
enum LimitSet {
    Route,
    Twin,
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
struct Comparison<L> {
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
    limits: L,
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
    match args.limits {
        LimitSet::Route => gate(args, RouteLimits::ORACLE),
        LimitSet::Twin => gate(args, TwinLimits::CALIBRATED),
    }
}

fn gate(args: Compare, limits: impl Limits) -> Result<ExitCode, Error> {
    let reference = decode::load(&args.reference)?;
    let actual = decode::load(&args.actual)?;
    let block_px = match (limits.block_pt(), args.scale) {
        (None, None) => None,
        (None, Some(_)) => {
            return Err(String::from(
                "--scale applies only to limits that judge blocks (--limits twin)",
            )
            .into());
        }
        (Some(block), Some(scale)) => Some(
            block
                .checked_mul(scale)
                .ok_or_else(|| format!("a {block} pt block at scale {scale} overflows"))?,
        ),
        (Some(_), None) => unreachable!("clap requires --scale for limits that judge blocks"),
    };
    let (score, heatmap) = score::compare(&reference, &actual, limits.shift_radius(), block_px)?;
    metrics::write_heatmap(&heatmap.rgb, heatmap.width, heatmap.height, &args.heatmap)?;
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

    use super::{Args, Command, LimitSet};

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
    fn route_limits_are_the_default_and_need_no_os_metadata() {
        let args = Args::try_parse_from(compare_args()).unwrap();
        let Command::Compare(compare) = args.command else {
            panic!("compare subcommand expected");
        };
        assert!(matches!(compare.limits, LimitSet::Route));
    }

    #[test]
    fn twin_limits_are_selected_by_name_and_need_the_capture_scale() {
        let mut args = compare_args();
        args.extend(["--limits", "twin"]);
        assert!(Args::try_parse_from(args.iter().copied()).is_err());
        args.extend(["--scale", "3"]);
        assert!(Args::try_parse_from(args.iter().copied()).is_ok());
    }
}
