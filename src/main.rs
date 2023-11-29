use std::{io::prelude::Write, path::PathBuf, time::Instant};

use clap::{Parser, ValueEnum, ValueHint};
use color_eyre::{
    eyre::{bail, ensure, Context},
    Result,
};
#[macro_use]
extern crate log;
use fastanvil::{RCoord, RegionFileLoader, RegionLoader};
use itertools::iproduct;
use rayon::prelude::*;
use region_scanner::*;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// The absolute path to the save folder of the world to scan.
    /// This is the folder the 'region' folder is in.
    /// Example: 'D:\Games\MultiMC\instances\FTB Presents Direwolf20
    /// 1.16\v.1.4.1\.minecraft\saves\MyTestWorld'
    #[arg(short, long, value_name = "FOLDER", value_hint=ValueHint::DirPath)]
    path: PathBuf,
    /// The dimension IDs to scan in the new format.
    /// Examples: 'minecraft:overworld', 'minecraft:the_nether',
    /// 'minecraft:the_end', 'jamd:mining'.
    #[arg(
        short,
        long,
        required = true,
        value_name = "DIMENSION_ID",
        num_args = 1..
    )]
    dims: Vec<String>,
    /// The zone to scan in every dimension, in regions, in the format of
    /// 'FROM_X,TO_X,FROM_Z,TO_Z' (separated either by commas or spaces).
    /// For example, '-1,1,-1,1' is a 2x2 square containing regions (-1,-1),
    /// (-1,0), (0,-1) and (0,0).
    #[arg(
        short,
        long,
        required = true,
        value_names = ["FROM_X", "TO_X", "FROM_Z", "TO_Z"],
        num_args = 1..=4, // necessary to make value_delimiter work here
        value_delimiter = ',',
        allow_hyphen_values = true
    )]
    zone: Vec<isize>,
    /// The format to export to
    #[arg(short, long, required=false, value_enum, default_value_t=ExportFormat::Jer)]
    format: ExportFormat,
    /// Number of worker threads to use for scanning dimensions. If
    /// set to zero, will be chosen automatically by rayon.
    #[arg(short, long, default_value_t = 0)]
    threads: usize,
}

#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, ValueEnum, Debug)]
enum ExportFormat {
    /// world-gen.json compatible with Just Enough Resources
    Jer,
    /// world-gen.csv file in CSV format - a row per each level
    /// and per each resource
    TallCSV,
}
fn init() -> Result<()> {
    if std::env::var_os("RUST_LOG").is_none() {
        std::env::set_var("RUST_LOG", "info");
    }
    pretty_env_logger::init();
    color_eyre::install()?;
    Ok(())
}
fn main() -> Result<()> {
    init()?;

    let args = Args::parse();
    ensure!(
        args.path.exists(),
        "It doesn't seem like the path `{}` exists!",
        args.path.display()
    );
    // Necessary check because this seems to not be possible to describe in clap v4
    // - it used to be num_values.
    ensure!(
        args.zone.len() == 4,
        "Wrong number of zone values! Expected: 4, got: {}. See --help or examples on the repo \
         for details.",
        args.zone.len()
    );
    let zone: Zone = Zone::from(args.zone);

    let mut paths_to_scan = vec![];
    for dimension in &args.dims {
        match get_path_from_dimension(dimension) {
            Some(suffix) => {
                let mut full_path = args.path.clone();
                full_path.push(suffix);
                paths_to_scan.push((dimension.as_str(), full_path.clone()));
                if !full_path.exists() {
                    bail!(
                        "Dimension name `{}` resolved to path `{}`, but this path doesn't exist! \
                         Perhaps you misspelled a dimension name (note in particular that that \
                         the vanilla dimensions are spelled `the_nether` and `the_end`), or tried \
                         to scan a dimension that wasn't generated yet for this world.",
                        dimension,
                        full_path.to_string_lossy()
                    );
                }
            }
            None => {
                bail!("Wasn't able to parse dimension: {}", dimension);
            }
        };
    }

    if args.threads != 0 {
        // Set rayon thread limit
        rayon::ThreadPoolBuilder::new()
            .num_threads(args.threads)
            .build_global()
            .context("Unable to set thread count!")?;
    }

    let results_by_dim = scan_multiple(&paths_to_scan, zone);
    let prefix = std::path::Path::new("output");
    std::fs::create_dir_all(prefix).unwrap();
    let (path, data) = match args.format {
        ExportFormat::Jer => {
            let json_string = generate_JER_json(&results_by_dim).unwrap();
            let path = prefix.join("world-gen.json");
            (path, json_string)
        }
        ExportFormat::TallCSV => {
            let csv_string = generate_tall_csv(&results_by_dim);
            let path = prefix.join("world-gen.csv");
            (path, csv_string)
        }
    };
    std::fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .create(true)
        .open(path)
        .unwrap()
        .write_all(data.as_bytes())
        .unwrap();
    Ok(())
}

fn scan_multiple(
    dim_paths: &[(&str, std::path::PathBuf)],
    zone: Zone,
) -> Vec<(BlockFrequencies, RegionVersion)> {
    let mut results_by_dim = vec![];
    for (dim, path) in dim_paths {
        info!(
            "Starting to scan dimension: {}, at {}.",
            dim,
            path.to_string_lossy()
        );
        match process_zone_in_folder(path, zone, dim) {
            DimensionScanResult::Ok(res) => results_by_dim.push(res),
            DimensionScanResult::NoRegionsPresent => {
                warn!(
                    "No regions were found in dimension {} located at '{}'. The zone specified \
                     has no regions, or the dimension isn't generated at all.",
                    dim,
                    path.display()
                )
            }
            DimensionScanResult::NoChunksFound => {
                warn!(
                    "Zero scannable chunks found in dimension {} located at '{}', despite regions \
                     being found. This might be caused by the world being of a minecraft version \
                     that's not supported, or it might be that the existing regions in the zone \
                     are all chunkless.",
                    dim,
                    path.display()
                )
            }
        }
    }
    results_by_dim
}
enum DimensionScanResult {
    Ok((BlockFrequencies, RegionVersion)),
    NoRegionsPresent,
    NoChunksFound,
}

fn process_zone_in_folder<S: AsRef<std::path::Path> + std::marker::Sync>(
    path: S,
    zone: Zone,
    dimension: &str,
) -> DimensionScanResult {
    let regions_num = (zone.1 - zone.0) * (zone.3 - zone.2);
    let indexes: Vec<(isize, isize)> = iproduct!(zone.0..zone.1, zone.2..zone.3).collect();
    let start = Instant::now();
    let verbose = false;
    // RegionFileLoader takes specifically a PathBuf, so we have to clone this one
    // for each thread.
    let regionfolder: std::path::PathBuf = std::path::PathBuf::from(path.as_ref());
    let version = determine_version(&mut RegionFileLoader::new(regionfolder.clone()), zone);
    info!(
        "World version detected as {}.",
        if matches!(version, RegionVersion::AtLeast118) {
            "at least 1.18"
        } else {
            "pre-1.18"
        }
    );

    let (total_freqs, valid_regions) = indexes
        .par_iter()
        .map(|(reg_x, reg_z)| {
            let s = regionfolder.clone();
            let regions = RegionFileLoader::new(s);

            match regions.region(RCoord(*reg_x), RCoord(*reg_z)) {
                Ok(Some(mut region)) => {
                    info!("Processing region ({},{}).", reg_x, reg_z);
                    (
                        RegionResult::Ok(count_frequencies(&mut region, verbose, dimension)),
                        1,
                    )
                }
                Ok(None) => {
                    info!("Region ({},{}) not found.", reg_x, reg_z);
                    (RegionResult::Ignore, 0)
                }
                Err(e) => {
                    warn!("Region ({reg_x},{reg_z}) failed to load! Error: {e:?}.");
                    (RegionResult::Ignore, 0)
                }
            }
        })
        .reduce(
            || (RegionResult::Ignore, 0),
            |(main, main_count), (other, other_count)| {
                let sum = match (main, other) {
                    (RegionResult::Ok(mut freqs1), RegionResult::Ok(freqs2)) => {
                        merge_frequencies_into(&mut freqs1, freqs2);
                        RegionResult::Ok(freqs1)
                    }
                    (RegionResult::Ok(freqs1), RegionResult::Ignore) => RegionResult::Ok(freqs1),
                    (RegionResult::Ignore, RegionResult::Ok(freqs2)) => RegionResult::Ok(freqs2),
                    (RegionResult::Ignore, RegionResult::Ignore) => RegionResult::Ignore,
                };
                (sum, main_count + other_count)
            },
        );
    let total_freqs = match total_freqs {
        RegionResult::Ok(freqs) => freqs,
        RegionResult::Ignore => return DimensionScanResult::NoRegionsPresent,
    };
    let elapsed_time = start.elapsed().as_secs_f32();
    // print_results(&total_freqs);
    info!(
        "Tried to scan {} regions. Succeeded in scanning {}.",
        regions_num, valid_regions
    );
    info!(
        "Nonempty chunks counted:{}, around {:.2}% of the zone specified.",
        total_freqs.chunks_counted,
        (total_freqs.chunks_counted as f64 / (regions_num * 1024) as f64) * 100.0
    );
    info!("Area on each layer:{}", total_freqs.area);
    info!("Blocks counted:{}", total_freqs.blocks_counted);
    info!(
        "Elapsed:{:.2}s for {} regions, average of {:.2}s per scanned region, or {:.2}s per 1024 \
         scanned chunks.",
        elapsed_time,
        regions_num,
        elapsed_time / valid_regions as f32,
        elapsed_time / (total_freqs.chunks_counted as f32) * 1024.0
    );
    if total_freqs.chunks_counted == 0 {
        return DimensionScanResult::NoChunksFound;
    }
    DimensionScanResult::Ok((total_freqs, version))
}

enum RegionResult {
    Ok(BlockFrequencies),
    Ignore,
}

#[allow(dead_code)]
fn print_results(result: &BlockFrequencies) {
    let max_len = result
        .frequencies
        .keys()
        .map(|x| x.len())
        .max()
        .expect("The result was empty!");
    for (name, nums) in &result.frequencies {
        if name.contains("ore") {
            let min_y = *nums.keys().min().unwrap_or(&0) as f64;
            let max_y = *nums.keys().max().unwrap_or(&256) as f64;
            let total: f64 = nums.values().sum();
            let average = total / (max_y - min_y);
            info!(
                "{:<width$}: {:>7.4}% ({:>9.3} per chunk)",
                name,
                average * 100.0,
                total * 256.0,
                width = ((max_len + 4) / 5) * 5
            );
        }
    }
}
