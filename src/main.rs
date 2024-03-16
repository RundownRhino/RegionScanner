use std::{io::prelude::Write, path::PathBuf, time::Instant};

use clap::{Parser, ValueEnum, ValueHint};
use color_eyre::{
    eyre::{bail, ensure, Context},
    Result,
};
#[macro_use]
extern crate log;
use fastanvil::{RCoord, RegionFileLoader, RegionLoader};
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
    /// (-1,0), (0,-1) and (0,0). If not provided, tries to scan all regions of
    /// each dimension.
    #[arg(
        short,
        long,
        required = false,
        value_names = ["FROM_X", "TO_X", "FROM_Z", "TO_Z"],
        num_args = 1..=4, // necessary to make value_delimiter work here
        value_delimiter = ',',
        allow_hyphen_values = true
    )]
    zone: Option<Vec<isize>>,
    /// The format to export to
    #[arg(short, long, required=false, value_enum, default_value_t=ExportFormat::Jer)]
    format: ExportFormat,
    /// Number of worker threads to use for scanning dimensions. If
    /// set to zero, will be chosen automatically by rayon.
    #[arg(short, long, default_value_t = 0)]
    threads: usize,
    /// If not none, only blocks with a normalized frequency above this value
    /// will be exported. Normalized frequency is the sum of frequencies by
    /// level divided by 255 (even in 1.18+ worlds which are higher than that).
    /// For example, a value of 0.01 means retain blocks more common that 1
    /// in 100 (which is ~655 such blocks per 255-height chunk). The default
    /// value is 1e-7, which is about 26 blocks pre 4096 chunks.
    /// Some comparisons: minecraft:emerald_ore is ~3e-6,
    /// minecraft:deepslate_emerald_ore (1.18) is ~2e-7,
    /// minecraft:ancient_debris is ~2e-5.
    #[arg(short, long, required = false, default_value = "1e-7")]
    only_blocks_above: Option<f64>,

    /// How to handle protochunks (chunks with a status other than
    /// minecraft:full, meaning they aren't fully generated).
    #[arg(long, required=false, value_enum, default_value_t=ProtoOption::Skip)]
    proto: ProtoOption,
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
    let zone: Option<Zone> = if let Some(coords) = args.zone {
        // Necessary check because this seems to not be possible to describe in clap v4
        // - it used to be num_values.
        ensure!(
            coords.len() == 4,
            "Wrong number of zone values! Expected: 4, got: {}. See --help or examples on the \
             repo for details.",
            coords.len()
        );
        Some(Zone::from(coords))
    } else {
        None
    };

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

    if let Some(x) = args.only_blocks_above {
        if x <= 0. {
            bail!(
                "Value of only_blocks_above must be positive if passed, got {}",
                x
            );
        }
    }

    if args.threads != 0 {
        // Set rayon thread limit
        rayon::ThreadPoolBuilder::new()
            .num_threads(args.threads)
            .build_global()
            .context("Unable to set thread count!")?;
    }

    let mut results_by_dim = scan_multiple(&paths_to_scan, zone, args.proto);

    if let Some(only_blocks_above) = args.only_blocks_above {
        let before: usize = results_by_dim
            .iter()
            .map(|(f, _)| f.frequencies.len())
            .sum();
        remove_too_rare(&mut results_by_dim, only_blocks_above);
        let after: usize = results_by_dim
            .iter()
            .map(|(f, _)| f.frequencies.len())
            .sum();
        info!(
            "Filtered results by normalized frequency. {} block-dim pairs out of {} were retained.",
            after, before
        );
    }

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
    zone: Option<Zone>,
    proto: ProtoOption,
) -> Vec<(BlockFrequencies, RegionVersion)> {
    let mut results_by_dim = vec![];
    for (dim, path) in dim_paths {
        info!(
            "Starting to scan dimension: {}, at {}.",
            dim,
            path.to_string_lossy()
        );
        match process_zone_in_folder(path, zone, dim, proto) {
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
    zone: Option<Zone>,
    dimension: &str,
    proto: ProtoOption,
) -> DimensionScanResult {
    // RegionFileLoader takes specifically a PathBuf, so we have to clone this one
    // for each thread.
    let regionfolder: std::path::PathBuf = std::path::PathBuf::from(path.as_ref());
    let loader = RegionFileLoader::new(regionfolder.clone());

    let coords = region_coords(&loader, zone);

    let start = Instant::now();
    let verbose = false;

    let version = determine_version(&loader, zone);
    info!(
        "World version detected as {}.",
        if matches!(version, RegionVersion::AtLeast118) {
            "at least 1.18"
        } else {
            "pre-1.18"
        }
    );

    let (total_freqs, valid_regions, seen_regions) = coords
        .par_iter()
        .map(|(x, z)| (x.0, z.0))
        .map(|(reg_x, reg_z)| {
            let s = regionfolder.clone();
            let regions = RegionFileLoader::new(s);

            match regions.region(RCoord(reg_x), RCoord(reg_z)) {
                Ok(Some(mut region)) => {
                    info!("Processing region ({}, {}).", reg_x, reg_z);
                    (
                        RegionResult::Ok(count_frequencies(&mut region, verbose, dimension, proto)),
                        1,
                        1usize,
                    )
                }
                Ok(None) => {
                    info!("Region ({}, {}) not found.", reg_x, reg_z);
                    (RegionResult::Ignore, 0, 1)
                }
                Err(e) => {
                    warn!("Region ({reg_x}, {reg_z}) failed to load! Error: {e:?}.");
                    (RegionResult::Ignore, 0, 1)
                }
            }
        })
        .reduce(
            || (RegionResult::Ignore, 0, 0),
            |(main, main_count, main_seen), (other, other_count, other_seen)| {
                let sum = match (main, other) {
                    (RegionResult::Ok(mut freqs1), RegionResult::Ok(freqs2)) => {
                        merge_frequencies_into(&mut freqs1, freqs2);
                        RegionResult::Ok(freqs1)
                    }
                    (RegionResult::Ok(freqs1), RegionResult::Ignore) => RegionResult::Ok(freqs1),
                    (RegionResult::Ignore, RegionResult::Ok(freqs2)) => RegionResult::Ok(freqs2),
                    (RegionResult::Ignore, RegionResult::Ignore) => RegionResult::Ignore,
                };
                (sum, main_count + other_count, main_seen + other_seen)
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
        zone.map(|z| z.size()).unwrap_or(seen_regions),
        valid_regions
    );
    if let Some(zone) = zone {
        info!(
            "Chunks scanned: {}, around {:.2}% of the zone specified.",
            total_freqs.chunks_counted,
            (total_freqs.chunks_counted as f64 / (zone.size() * 1024) as f64) * 100.0
        );
    } else {
        info!(
            "Chunks scanned: {}, around {:.2}% of the area of the regions found.",
            total_freqs.chunks_counted,
            (total_freqs.chunks_counted as f64 / (seen_regions * 1024) as f64) * 100.0
        );
    }

    match proto {
        ProtoOption::Skip => info!("{} protochunks were skipped.", total_freqs.protochunks_seen),
        ProtoOption::Include => {
            info!(
                "{} of the scanned chunks were protochunks.",
                total_freqs.protochunks_seen
            )
        }
        ProtoOption::OnlyProto => info!("All of the scanned chunks were protochunks"),
    }
    info!("Area on each layer: {}", total_freqs.area);
    info!("Blocks counted: {}", total_freqs.blocks_counted);
    info!(
        "Elapsed: {:.2}s, average of {:.2}s per scanned region, or {:.2}s per 1024 scanned chunks.",
        elapsed_time,
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
