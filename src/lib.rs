mod utils;

use std::{
    collections::{hash_map::Entry, HashMap, HashSet},
    fmt::Write,
    fs::File,
    path::{Path, PathBuf},
    sync::Mutex,
};

use utils::*;
#[macro_use]
extern crate log;
use fastanvil::{Chunk, JavaChunk, RCoord, Region, RegionFileLoader, RegionLoader};
use itertools::iproduct;
use serde::{Deserialize, Serialize};

pub fn count_blocks(
    region: &mut Region<File>,
    verbose: bool,
    dimension: &str,
    proto: ProtoOption,
) -> BlockCounts {
    let mut chunks_counted = 0;
    let mut protochunks_seen = 0;
    let mut blocks_counted: u64 = 0;
    let mut counts: HashMap<String, HashMap<isize, u64>> = HashMap::new();
    let mut closure = |xpos: usize, zpos: usize, chunk_processed: JavaChunk| {
        if verbose && chunks_counted % 100 == 0 {
            info!(
                "Handling chunk number {} at position ({},{})",
                chunks_counted + 1,
                xpos,
                zpos
            );
        }
        // The block data is stored in sections by y, so we iterate by y least often.
        // Inside a section, x is the fastest-changing index. Hence, order yzx.
        for (y, z, x) in iproduct!(chunk_processed.y_range(), 0..16, 0..16) {
            if let Some(block) = chunk_processed.block(x, y, z) {
                let block_entry = counts.entry(block.name().to_string());
                let count_entry = block_entry.or_default().entry(y).or_insert(0);
                *count_entry += 1;
            }
            blocks_counted += 1;
        }
        chunks_counted += 1;
    };

    for data in chunks(region).flatten() {
        use ProtoOption::*;
        // This silently skips chunks that fail to deserialise.
        if let Ok(c) = JavaChunk::from_bytes(&data.data) {
            // See https://minecraft.wiki/w/Chunk_format
            // It seems pre-1.18, "full" is used instead, so allow both.
            let chunk_state = c.status();
            let is_full = chunk_state == "minecraft:full" || chunk_state == "full";
            if !is_full {
                protochunks_seen += 1;
                if proto == Skip {
                    continue;
                }
            }
            // otherwise it's a full chunk
            else if proto == OnlyProto {
                continue;
            }
            closure(data.x, data.z, c);
        }
    }
    BlockCounts {
        counts,
        blocks_counted,
        chunks_counted,
        protochunks_seen,
        dimension: dimension.to_string(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum ProtoOption {
    /// Protochunks will be skipped
    Skip,
    /// Protochunks will be included in the scan
    Include,
    /// *Only* protochunks will be scanned (useful for testing).
    OnlyProto,
}

pub struct BlockCounts {
    pub counts: HashMap<String, HashMap<isize, u64>>,
    pub blocks_counted: u64,
    pub chunks_counted: usize,
    pub protochunks_seen: usize,
    pub dimension: String,
}
pub struct BlockFrequencies {
    // Remember to update merge_frequencies_into when adding fields!
    pub frequencies: HashMap<String, HashMap<isize, f64>>,
    pub blocks_counted: u64,
    pub chunks_counted: usize,
    /// For ProtoOption::Skip these were skipped, for Include they are part of
    /// the counted, for OnlyProto should be equal to chunks_counted.
    pub protochunks_seen: usize,
    pub area: u64,
    pub dimension: String,
}
impl BlockFrequencies {
    pub fn empty(dimension: String) -> BlockFrequencies {
        BlockFrequencies {
            frequencies: HashMap::new(),
            blocks_counted: 0,
            chunks_counted: 0,
            protochunks_seen: 0,
            area: 0,
            dimension,
        }
    }
}
#[derive(Copy, Clone)]
pub struct Zone {
    pub from_x: isize,
    pub to_x: isize,
    pub from_z: isize,
    pub to_z: isize,
}

impl Zone {
    pub fn new(from_x: isize, to_x: isize, from_z: isize, to_z: isize) -> Self {
        if to_x <= from_x {
            panic!(
                "Tried to create a Zone with from_x={from_x}, to_x={to_x}. This is invalid - to_x \
                 must be larger than from_x for the zone to be nonempty. Perhaps the order of \
                 arguments is wrong?"
            );
        }
        Self {
            from_x,
            to_x,
            from_z,
            to_z,
        }
    }

    pub fn size(&self) -> usize {
        (self.to_x - self.from_x) as usize * (self.to_z - self.from_z) as usize
    }
}

impl From<Vec<isize>> for Zone {
    fn from(vec: Vec<isize>) -> Self {
        if vec.len() < 4 {
            panic!("Vector too small to convert to a Zone:{:?}", vec);
        }
        Zone::new(vec[0], vec[1], vec[2], vec[3])
    }
}
#[derive(Clone, Copy, Debug)]
pub enum RegionVersion {
    Pre118,
    AtLeast118,
}
/// Determines the version of a world by checking the first nonempty region it
/// finds in the zone provided (or all the regions in the loader).
pub fn determine_version(loader: &RegionFileLoader, zone: Option<Zone>) -> RegionVersion {
    use fastanvil::JavaChunk as JavaChunkEnum;
    for mut region in iter_regions(loader, zone) {
        if let Some(c) = chunks(&mut region)
            .find_map(|data| data.and_then(|x| JavaChunkEnum::from_bytes(&x.data).ok()))
        {
            return match c {
                JavaChunkEnum::Post18(_) => RegionVersion::AtLeast118,
                JavaChunkEnum::Pre18(_) => RegionVersion::Pre118,
                JavaChunkEnum::Pre13(_) => RegionVersion::Pre118,
            };
        }
    }
    panic!(
        "Was unable to find a single chunk in a single region in the zone provided that was \
         readable!"
    );
}

pub fn region_coords(loader: &RegionFileLoader, zone: Option<Zone>) -> Vec<(RCoord, RCoord)> {
    if let Some(zone) = zone {
        iproduct!(zone.from_x..zone.to_x, zone.from_z..zone.to_z)
            .map(|(x, z)| (RCoord(x), RCoord(z)))
            .collect()
    } else {
        loader.list().unwrap()
    }
}

/// Iterates over the regions in a zone, or all regions in the loader. Ignores
/// regions that fail to load, which may or may not be a good idea
pub fn iter_regions(
    loader: &RegionFileLoader,
    zone: Option<Zone>,
) -> impl Iterator<Item = Region<File>> + '_ {
    region_coords(loader, zone)
        .into_iter()
        .filter_map(|(reg_x, reg_z)| loader.region(reg_x, reg_z).ok().flatten())
}

pub fn count_frequencies(
    region: &mut Region<File>,
    verbose: bool,
    dimension: &str,
    proto: ProtoOption,
) -> BlockFrequencies {
    let counting_results = count_blocks(region, verbose, dimension, proto);
    let area: u64 = (16 * 16) * counting_results.chunks_counted as u64;
    let mut frequencies: HashMap<String, HashMap<isize, f64>> = HashMap::new();
    let d_area = area as f64;
    for (name, nums) in counting_results.counts {
        frequencies.insert(
            name,
            nums.iter()
                .map(|(&y, &count)| (y, count as f64 / d_area))
                .collect(),
        );
    }
    BlockFrequencies {
        frequencies,
        blocks_counted: counting_results.blocks_counted,
        chunks_counted: counting_results.chunks_counted,
        protochunks_seen: counting_results.protochunks_seen,
        area,
        dimension: counting_results.dimension,
    }
}

pub fn merge_frequencies_into(main: &mut BlockFrequencies, other: BlockFrequencies) {
    for (name, freq) in other.frequencies {
        match main.frequencies.entry(name) {
            Entry::Occupied(mut a) => {
                let total_area: f64 = (main.area + other.area) as f64;
                let alpha: f64 = main.area as f64 / total_area;
                counts_add_weighted(a.get_mut(), &freq, alpha);
            }
            Entry::Vacant(a) => {
                a.insert(freq);
            }
        }
    }
    main.area += other.area;
    main.blocks_counted += other.blocks_counted;
    main.chunks_counted += other.chunks_counted;
    main.protochunks_seen += other.protochunks_seen;
}
pub fn counts_add_weighted(a: &mut HashMap<isize, f64>, b: &HashMap<isize, f64>, a_weight: f64) {
    assert!(
        (0.0..=1.0).contains(&a_weight),
        "Weight is not in the [0,1] range!"
    );

    let b_weight = 1.0 - a_weight;
    let keys: HashSet<isize> = a.keys().chain(b.keys()).cloned().collect();
    for key in keys {
        let a_val = *a.get(&key).unwrap_or(&0.0) * a_weight;
        let b_val = *b.get(&key).unwrap_or(&0.0) * b_weight;
        a.insert(key, a_val + b_val);
    }
}
#[allow(non_snake_case)]
pub fn generate_JER_json(
    frequency_data: &[(BlockFrequencies, RegionVersion)],
) -> Result<String, serde_json::Error> {
    let mut distrib_list: Vec<BlockJERDistributionData> = vec![];
    for (freq_data, version) in frequency_data {
        for (name, freqs) in &freq_data.frequencies {
            if freqs.is_empty() {
                continue;
            }
            let distrib = freqs_to_distrib(freqs, *version, &freq_data.dimension, name);
            if distrib.is_empty() {
                continue;
            }
            distrib_list.push(BlockJERDistributionData {
                block: name.clone(),
                distrib,
                silktouch: false,
                dim: freq_data.dimension.clone().to_string(),
            });
        }
    }
    serde_json::to_string_pretty(&distrib_list)
}

pub fn generate_tall_csv(frequency_data: &[(BlockFrequencies, RegionVersion)]) -> String {
    let mut res = String::new();
    res.write_str("dim,block,level,freq\n").unwrap();
    for (freq_data, _version) in frequency_data {
        for (name, freqs) in &freq_data.frequencies {
            if freqs.is_empty() {
                continue;
            }
            let min_y = *freqs.keys().min().unwrap();
            let max_y = *freqs.keys().max().unwrap();
            for y in min_y..=max_y {
                res.write_str(&format!(
                    "{},{},{},{}\n",
                    freq_data.dimension,
                    name,
                    y,
                    freqs.get(&y).unwrap_or(&0f64)
                ))
                .expect("Error when assembling CSV");
            }
        }
    }
    res
}

fn freqs_to_distrib(
    freqs: &HashMap<isize, f64>,
    version: RegionVersion,
    dimension: &str,
    name: &str,
) -> String {
    assert!(!freqs.is_empty(), "Got an empty distribution!");
    let mut distrib = String::new();

    // JER for 1.18+ stores the levels with an offset of 64 - that way levels start
    // from 0 inclusive regardless of version.
    let offset = match version {
        RegionVersion::Pre118 => 0,
        RegionVersion::AtLeast118 => 64,
    };
    // We always mention all values from the very bottom of the world, otherwise JER
    // plots for rare ores can look bad.
    let depth_limit = -offset;
    let max_jer_height = 255;
    let min_y = *freqs.keys().min().unwrap();
    let max_y = *freqs.keys().max().unwrap();

    // It *is* possible for a modded dimension to be below that limit.
    // However, for JER export we ignore it, since JER won't be able to
    // render it as a plot anyway. See issue #11 for details.
    // Similarly, we discard data about y=255, as the JER loads the frequencies as
    // an array of size 320 and will raise an error on bigger ones. See issue #16.
    static DIMENSIONS_LIMITS_EXCEEDED: Mutex<Vec<String>> = Mutex::new(vec![]);
    if min_y < depth_limit || max_y > max_jer_height {
        let mut cache = DIMENSIONS_LIMITS_EXCEEDED.lock().unwrap();
        if !cache.iter().any(|x| x == dimension) {
            warn!(
                "Block kind {name} for dimension {dimension} exceeded the dimension height limits \
                 of {depth_limit} to {max_jer_height}: the lowest block of this kind was at \
                 y={min_y} and the highest at y={max_y}. Frequencies outside the height limits \
                 will be omitted when exporting, as JER doesn't support them. Use another export \
                 format to avoid this limitation. Further occurences of this warning for this \
                 dimension will be at level TRACE."
            );
            cache.push(dimension.to_owned());
        } else {
            trace!(
                "Block kind {name} for dimension {dimension} exceeded the dimension height limits \
                 of {depth_limit} to {max_jer_height}: the lowest block of this kind was at \
                 y={min_y} and the highest at y={max_y}."
            );
        }
    }

    for y in depth_limit..=isize::min(max_y, max_jer_height) {
        let value = *freqs.get(&y).unwrap_or(&0f64);
        distrib.push_str(&format!("{},{};", (y + offset) as u16, value));
    }
    distrib
}
#[derive(Serialize, Deserialize)]
pub struct BlockJERDistributionData {
    block: String,
    distrib: String,
    silktouch: bool,
    dim: String,
}

pub fn get_path_from_dimension(dimension: &str) -> Option<PathBuf> {
    if dimension == "minecraft:overworld" {
        Some(Path::new(r"region").to_path_buf())
    } else if dimension == "minecraft:the_nether" {
        Some(Path::new(r"DIM-1/region").to_path_buf())
    } else if dimension == "minecraft:the_end" {
        Some(Path::new(r"DIM1/region").to_path_buf())
    } else {
        let parts: Vec<&str> = dimension.split(':').collect();
        if parts.len() != 2 {
            None
        } else {
            let mut result = PathBuf::from(r"dimensions/");
            result.push(parts[0]);
            result.push(parts[1]);
            result.push("region");
            Some(result)
        }
    }
}

#[test]
fn test_dim_to_path_conversions() {
    let correct_results = [
        ("minecraft:overworld", "region"),
        ("minecraft:the_end", "DIM1/region"),
        ("minecraft:the_nether", "DIM-1/region"),
        (
            "appliedenergistics2:spatial_storage",
            r"dimensions/appliedenergistics2\spatial_storage\region",
        ),
    ];
    let mut wrong = vec![];
    for (inp, out) in correct_results {
        let generated = get_path_from_dimension(inp).map(|x| x.to_str().unwrap().to_owned());

        if generated.is_none() || generated.as_ref().unwrap() != out {
            wrong.push((inp, out, generated));
        }
    }
    if !wrong.is_empty() {
        let mut panic_str = format!(
            "Of {} conversion tests, {} failed:",
            correct_results.len(),
            wrong.len()
        );
        for (inp, out, generated) in wrong {
            panic_str.push_str(&format!(
                "\nInput: '{}', expected: '{}', got: '{}'",
                inp,
                generated.unwrap_or_else(|| "<invalid input>".to_string()),
                out
            ));
        }
        panic!("{}", panic_str);
    }
}

pub fn remove_too_rare(results_by_dim: &mut [(BlockFrequencies, RegionVersion)], cutoff: f64) {
    if cutoff <= 0. {
        panic!("Cutoff must be positive, got {}", cutoff);
    }
    // Always 255, even in 1.18+ worlds - otherwise this
    // metric would change for the same world between versions.
    let world_height = 255f64;
    for (freqs, _) in results_by_dim.iter_mut() {
        freqs.frequencies.retain(|k, v: &mut HashMap<isize, f64>| {
            let normalized_frequency = v.values().sum::<f64>() / world_height;
            if normalized_frequency >= cutoff {
                true
            } else {
                trace!(
                    "Dropping record ({}, {}) with normalized frequency {:.2e}.",
                    &freqs.dimension,
                    k,
                    normalized_frequency
                );
                false
            }
        });
    }
}
