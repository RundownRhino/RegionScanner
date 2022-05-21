use fastanvil::JavaChunk;
use fastanvil::{Chunk, Region};
use std::collections::HashSet;

use itertools::iproduct;
use serde::{Deserialize, Serialize};
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::fs::File;
use std::path::{Path, PathBuf};

pub fn count_blocks(region: &mut Region<File>, verbose: bool, dimension: &str) -> BlockCounts {
    let mut chunks_counted: usize = 0;
    let mut blocks_counted: u64 = 0;
    let mut counts: HashMap<String, HashMap<isize, u64>> = HashMap::new();
    let mut closure = |xpos: usize, zpos: usize, chunk_processed: JavaChunk| {
        if verbose && chunks_counted % 100 == 0 {
            println!(
                "Handling chunk number {} at position ({},{})",
                chunks_counted + 1,
                xpos,
                zpos
            );
        }
        for (x, y, z) in iproduct!(0..16, chunk_processed.y_range(), 0..16) {
            if let Some(block) = chunk_processed.block(x, y, z) {
                let block_entry = counts.entry(block.name().to_string());
                let count_entry = block_entry
                    .or_insert_with(|| HashMap::new())
                    .entry(y)
                    .or_insert(0);
                *count_entry += 1;
            }
            blocks_counted += 1;
        }
        chunks_counted += 1;
    };
    for (x, z) in iproduct!(0..32, 0..32) {
        if let Some(c) = region
            .read_chunk(x, z)
            .unwrap()
            // This silently skips chunks that fail to deserialise.
            .and_then(|data| JavaChunk::from_bytes(&data).ok())
        {
            closure(x as usize, z as usize, c);
        }
    }
    BlockCounts {
        counts,
        blocks_counted,
        chunks_counted,
        dimension: dimension.to_string(),
    }
}

pub struct BlockCounts {
    pub counts: HashMap<String, HashMap<isize, u64>>,
    pub blocks_counted: u64,
    pub chunks_counted: usize,
    pub dimension: String,
}
pub struct BlockFrequencies {
    pub frequencies: HashMap<String, HashMap<isize, f64>>,
    pub blocks_counted: u64,
    pub chunks_counted: usize,
    pub area: u64,
    pub dimension: String,
}
impl BlockFrequencies {
    pub fn empty(dimension: String) -> BlockFrequencies {
        BlockFrequencies {
            frequencies: HashMap::new(),
            blocks_counted: 0,
            chunks_counted: 0,
            area: 0,
            dimension,
        }
    }
}
pub fn count_frequencies(
    region: &mut Region<File>,
    verbose: bool,
    dimension: &str,
) -> BlockFrequencies {
    let counting_results = count_blocks(region, verbose, dimension);
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
                vector_add_weighted(a.get_mut(), &freq, alpha);
            }
            Entry::Vacant(a) => {
                a.insert(freq);
            }
        }
    }
    main.area += other.area;
    main.blocks_counted += other.blocks_counted;
    main.chunks_counted += other.chunks_counted;
}
pub fn vector_add_weighted(
    a: &mut HashMap<isize, f64>,
    b: &HashMap<isize, f64>,
    a_weight: f64,
) {
    if !(0.0..=1.0).contains(&a_weight) {
        panic!("Weight is not in the [0,1] range!");
    }
    let b_weight = 1.0 - a_weight;
    let keys: HashSet<isize> = a.keys().chain(b.keys()).cloned().collect();
    for key in keys {
        let a_val = *a.get(&key).unwrap_or(&0.0) * a_weight;
        let b_val = *b.get(&key).unwrap_or(&0.0) * b_weight;
        a.insert(key, a_val + b_val);
    }
}
#[allow(non_snake_case)]
pub fn generate_JER_json(freq_datas: &[BlockFrequencies]) -> Result<String, serde_json::Error> {
    let mut distrib_list: Vec<BlockJERDistributionData> = vec![];
    for freq_data in freq_datas {
        for (name, freqs) in &freq_data.frequencies {
            distrib_list.push(BlockJERDistributionData {
                block: name.clone(),
                distrib: freqs_to_distrib(freqs),
                silktouch: false,
                dim: freq_data.dimension.clone().to_string(),
            });
        }
    }
    serde_json::to_string_pretty(&distrib_list)
}
fn freqs_to_distrib(freqs: &HashMap<isize, f64>) -> String {
    let mut distrib = String::new();
    freqs
        .iter()
        .map(|(y, value)| format!("{},{};", y, value))
        .for_each(|x| distrib.push_str(&x));
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
            result.push(parts.get(0)?);
            result.push(parts.get(1)?);
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
