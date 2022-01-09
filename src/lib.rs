// #[cfg(test)]
// mod tests {
//     #[test]
//     fn it_works() {
//         assert_eq!(2 + 2, 4);
//     }
// }
use fastanvil::pre18::JavaChunk;
use fastanvil::{CCoord, Chunk, Region};

use itertools::iproduct;
use serde::{Deserialize, Serialize};
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::ops::{Add, Mul};
use std::path::{Path, PathBuf};

pub fn count_blocks(region: &dyn Region<JavaChunk>, verbose: bool, dimension: &str) -> BlockCounts {
    let mut chunks_counted: usize = 0;
    let mut blocks_counted: u64 = 0;
    let mut counts: HashMap<String, Vec<u64>> = HashMap::new();
    let mut closure = |xpos: usize, zpos: usize, chunk_processed: JavaChunk| {
        if verbose && chunks_counted % 100 == 0 {
            println!(
                "Handling chunk number {} at position ({},{})",
                chunks_counted + 1,
                xpos,
                zpos
            );
        }
        for y in 0..256 {
            for (x, z) in iproduct!(0..16, 0..16) {
                let block = chunk_processed.block(x, y, z);
                if let Some(a) = block {
                    counts
                        .entry(a.name().to_string())
                        .or_insert_with(|| vec![0; 256])[y as usize] += 1;
                }
                blocks_counted += 1;
            }
        }
        chunks_counted += 1;
    };
    for x in 0..32 {
        for z in 0..32 {
            if let Some(c) = region.chunk(CCoord(x), CCoord(z)) {
                closure(x as usize, z as usize, c);
            }
        }
    }
    BlockCounts {
        counts,
        //elapsed_time,
        blocks_counted,
        chunks_counted,
        dimension: dimension.to_string(),
    }
}

pub struct BlockCounts {
    pub counts: HashMap<String, Vec<u64>>,
    //pub elapsed_time: f32,
    pub blocks_counted: u64,
    pub chunks_counted: usize,
    pub dimension: String,
}
pub struct BlockFrequencies {
    pub frequencies: HashMap<String, Vec<f64>>,
    //pub elapsed_time: f32,
    pub blocks_counted: u64,
    pub chunks_counted: usize,
    pub area: u64,
    pub dimension: String,
}
impl BlockFrequencies {
    pub fn empty(dimension: String) -> BlockFrequencies {
        BlockFrequencies {
            frequencies: HashMap::new(),
            //elapsed_time: 0.0,
            blocks_counted: 0,
            chunks_counted: 0,
            area: 0,
            dimension,
        }
    }
}
pub fn count_frequencies(
    region: &dyn Region<JavaChunk>,
    verbose: bool,
    dimension: &str,
) -> BlockFrequencies {
    let counting_results = count_blocks(region, verbose, dimension);
    let area: u64 = (16 * 16) * counting_results.chunks_counted as u64;
    let mut frequencies: HashMap<String, Vec<f64>> = HashMap::new();
    let d_area = area as f64;
    for (name, nums) in counting_results.counts {
        frequencies.insert(name, nums.iter().map(|&x| x as f64 / d_area).collect());
    }
    BlockFrequencies {
        frequencies,
        //elapsed_time: counting_results.elapsed_time,
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
    //main.elapsed_time += other.elapsed_time;
    main.blocks_counted += other.blocks_counted;
    main.chunks_counted += other.chunks_counted;
}
pub fn vector_add_weighted<T: Add<T, Output = T> + Mul<f64, Output = T> + Copy>(
    a: &mut Vec<T>,
    b: &[T],
    a_weight: f64,
) {
    if !(0.0..=1.0).contains(&a_weight) {
        panic!("Weight is not in the [0,1] range!");
    }
    let b_weight = 1.0 - a_weight;
    for (first, second) in a.iter_mut().zip(b) {
        *first = *first * a_weight + *second * b_weight;
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
fn freqs_to_distrib(freqs: &[f64]) -> String {
    let mut distrib = String::new();
    freqs
        .iter()
        .enumerate()
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
    for s in &[
        "minecraft:overworld",
        "minecraft:the_end",
        "minecraft:the_nether",
        "appliedenergistics2:spatial_storage",
    ] {
        println!(
            "{} : {}",
            s,
            get_path_from_dimension(s).unwrap().to_str().unwrap()
        );
    }
}
