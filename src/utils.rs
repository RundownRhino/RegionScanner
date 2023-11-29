use std::fs::File;

use fastanvil::{ChunkData, Region};
use itertools::iproduct;

/// Used instead of Region.iter(), which skips over missing chunks
pub fn chunks(region: &mut Region<File>) -> impl Iterator<Item = Option<ChunkData>> + '_ {
    // x should be the first-changing index - see header_pos in fastanvil
    iproduct!(0..32, 0..32).map(|(chunk_z, chunk_x)| {
        region
            .read_chunk(chunk_x, chunk_z)
            .unwrap()
            .map(|data| ChunkData {
                x: chunk_x,
                z: chunk_z,
                data,
            })
    })
}
