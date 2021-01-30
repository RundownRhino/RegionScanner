use clap::{crate_authors, crate_description, crate_version, App, Arg};
use fastanvil::Region;
use itertools::iproduct;
use rayon::prelude::*;
use region_scanner::*;
use std::io::prelude::Write;
use std::time::Instant;

fn main() {
    let matches = App::new("Region scanner")
        .version(crate_version!())
        .author(crate_authors!())
        .about(crate_description!())
        .arg(Arg::with_name("path")
            .long("path")
            .value_name("FOLDER")
            .help(r"The absolute path to the save folder of the world in question.
            This is the folder the 'region' folder is in.
            Example: 'D:\Games\MultiMC\instances\FTB Presents Direwolf20 1.16 v.1.4.1\.minecraft\saves\MyTestWorld'")
            .takes_value(true)
            .required(true)
        ) 
        .arg(Arg::with_name("dims")
            .long("dims")
            .value_name("DIMENSION_ID")
            .help("The dimension ID in the new format.
            Examples: 'minecraft:overworld', 'minecraft:the_nether', 'minecraft:the_end','jamd:mining'.")
            .takes_value(true)
            .required(true)
            .min_values(1)
        )
        .arg(Arg::with_name("zone")
            .long("zone")
            .value_name("ZONE")
            .help("The zone to scan in every dimension, in regions, in the format of 'from_x,to_x,from_z,to_z'.
            For example, '-1,1,-1,1' is a 2x2 square containing regions (-1,-1), (-1,0), (0,-1) and (0,0).")
            .takes_value(true)
            .required(true)
            .number_of_values(4)
            .value_delimiter(",")
            .allow_hyphen_values(true)
        )
        .get_matches();
    //println!("{:?}", matches);
    //panic!();
    let save_str = matches.value_of("path").unwrap();
    let dims_to_scan:Vec<&str> = matches.values_of("dims").unwrap().collect();
    if !std::path::Path::new(save_str).exists(){
        panic!("It doesn't seem like the path {} exists!",save_str);
    }
    let save_path = std::path::PathBuf::from(save_str);
    let zone_values: Vec<isize> = matches.values_of("zone").unwrap().map(|s| s.parse().unwrap()).collect();
    if zone_values.len() != 4{
        panic!("Wrong number of zone values! Expected: 4, got : {}",zone_values.len());
    }
    let zone: Zone = Zone::from(zone_values);
    let mut paths_to_scan = vec![];
    for dimension in &dims_to_scan {
        match get_path_from_dimension(dimension) {
            Some(suffix) => {
                let mut full_path = save_path.clone();
                full_path.push(suffix);
                paths_to_scan.push((*dimension, full_path))
            }
            None => {
                panic!("Wasn't able to parse dimension: {}", dimension);
            }
        };
    }
    let freqs_by_dim = scan_multiple(&paths_to_scan, zone);
    let json_string = generate_JER_json(&freqs_by_dim).unwrap();
    let path = std::path::Path::new("output/world-gen.json");
    let prefix = path.parent().unwrap();
    std::fs::create_dir_all(prefix).unwrap();
    std::fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .create(true)
        .open(path)
        .unwrap()
        .write_all(json_string.as_bytes())
        .unwrap();
}

fn scan_multiple(
    dim_paths: &[(&str, std::path::PathBuf)],
    zone: Zone,
) -> Vec<BlockFrequencies> {
    let mut freqs_by_dim = vec![];
    for (dim, path) in dim_paths {
        println!(
            "\nStarting to scan dimension: {}, at {}.",
            dim,
            path.to_string_lossy()
        );
        match process_zone_in_folder(path, zone, dim) {
            DimensionScanResult::Ok(freqs) => freqs_by_dim.push(freqs),
            DimensionScanResult::NoRegionsPresent => println!("No regions were found!"),
        }
    }
    freqs_by_dim
}
#[derive(Copy,Clone)]
struct Zone(isize,isize,isize,isize);
impl From<Vec<isize>> for Zone{
    fn from(vec: Vec<isize>) -> Self{
        if vec.len()<4{panic!("Vector too small to convert to a Zone:{:?}",vec);}
        Zone(*vec.get(0).unwrap(),*vec.get(1).unwrap(),*vec.get(2).unwrap(),*vec.get(3).unwrap())
    }
}
enum DimensionScanResult {
    Ok(BlockFrequencies),
    NoRegionsPresent,
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
    let regionfolder: std::path::PathBuf = std::path::PathBuf::from(path.as_ref());

    let (total_freqs, valid_regions) = indexes
        .par_iter()
        .map(|(reg_x, reg_z)| {
            let mut s = regionfolder.clone();
            s.push(format!(r"r.{}.{}.mca", reg_x, reg_z));
            //let s = format!(r"{}\r.{}.{}.mca", regionfolder, reg_x, reg_z);
            let file = std::fs::File::open(&s);
            match file {
                Ok(file) => {
                    println!("Processing region ({},{}).", reg_x, reg_z);
                    let mut region = Region::new(file);
                    (RegionResult::Ok(count_frequencies(&mut region, verbose,dimension)), 1)
                }
                Err(e) => {
                    if let std::io::ErrorKind::NotFound = e.kind() {
                        println!("Region ({},{}) not found.", reg_x, reg_z)
                    } else {
                        println!("Found region, but wasn't able to open: {}.\nGot the following error:{}", s.to_string_lossy(), e);
                    }
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
    //print_results(&total_freqs);
    println!(
        "Tried to scan {} regions. Succeeded in scanning {}.",
        regions_num, valid_regions
    );
    println!(
        "Nonempty chunks counted:{}, around {:.2}% of the zone specified.",
        total_freqs.chunks_counted,
        (total_freqs.chunks_counted as f64 / (regions_num * 1024) as f64) * 100.0
    );
    println!("Area on each layer:{}", total_freqs.area);
    println!("Blocks counted:{}", total_freqs.blocks_counted);
    println!(
        "Elapsed:{:.2}s for {} regions, average of {:.2}s per scanned region, or {:.2}s per 1024 scanned chunks.",
        elapsed_time,
        regions_num,
        elapsed_time / valid_regions as f32,
        elapsed_time / (total_freqs.chunks_counted as f32) * 1024.0
    );
    DimensionScanResult::Ok(total_freqs)
}

enum RegionResult {
    Ok(BlockFrequencies),
    Ignore,
}

fn print_results(result: &BlockFrequencies) {
    let max_len = result
        .frequencies
        .keys()
        .map(|x| x.len())
        .max()
        .expect("The result was empty!");
    for (name, nums) in &result.frequencies {
        if name.contains("ore") {
            let total: f64 = nums.iter().sum();
            let average = total / 256.0;
            println!(
                "{:<width$}: {:>7.4}% ({:>9.3} per chunk)",
                name,
                average * 100.0,
                total * 256.0,
                width = ((max_len + 4) / 5) * 5
            );
        }
    }
}
