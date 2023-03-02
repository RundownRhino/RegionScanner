# RegionScanner
A CLI program to scan Minecraft region files and create Just Enough Resources world-gen.json files (or some other formats) from the results. Tested and works on 1.13-1.18 inclusive. On 1.12 and below works only on vanilla worlds (and if you're on 1.12.2 and below, you should be able to use JER's own profiling feature instead).

# Installation
The repo automatically builds binaries for multiple different targets via Github Actions, so unless you're using an obscure platform, you should be able to just grab a build for yours from [Releases](https://github.com/RundownRhino/RegionScanner/releases/latest). Extract the executable from the archive and use.

If you're on Windows and not sure which to pick, you most likely want `x86_64-pc-windows-msvc`. If you're on a *32bit* Windows, `i686-pc-windows-msvc`.

# Usage:
`region_scanner --dims <DIMENSION_ID> --path <FOLDER> --zone <ZONE>`.

See `--help` for help. Excerpt:
```
Options:
  -p, --path <FOLDER>
          The absolute path to the save folder of the world to scan. This is the folder the 'region' folder is in. Example: 'D:\Games\MultiMC\instances\FTB Presents Direwolf20 1.16\v.1.4.1\.minecraft\saves\MyTestWorld'

  -d, --dims <DIMENSION_ID>...
          The dimension IDs to scan in the new format. Examples: 'minecraft:overworld', 'minecraft:the_nether', 'minecraft:the_end', 'jamd:mining'

  -z, --zone <FROM_X> <TO_X> <FROM_Z> <TO_Z>
          The zone to scan in every dimension, in regions, in the format of 'FROM_X,TO_X,FROM_Z,TO_Z' (separated either by commas or spaces). For example, '-1,1,-1,1' is a 2x2 square containing regions (-1,-1), (-1,0), (0,-1) and (0,0)

  -f, --format <FORMAT>
          The format to export to

          [default: jer]

          Possible values:
          - jer:      world-gen.json compatible with Just Enough Resources
          - tall-csv: world-gen.csv file in CSV format - a row per each level and per each resource

  -t, --threads <THREADS>
          Number of worker threads to use for scanning dimensions. If set to zero, will be chosen automatically by rayon

          [default: 0]
```

Example command: `region_scanner.exe --path "D:\Games\MultiMC\instances\FTB Presents Direwolf20 1.16 v.1.4.1\.minecraft\saves\MyTestWorld" --dims minecraft:overworld minecraft:the_nether minecraft:the_end --zone -1,1,-1,1`.

# Detailed instructions for generating a JER file:
1. Download the executable from releases and place it wherever you want, preferably in a folder of its own. You'll also want a way to efficiently pregenerate the world, like [Chunk Pregenerator](https://www.curseforge.com/minecraft/mc-mods/chunkpregenerator).
2. Make a new world. Pregenerate a large area around the world origin - for example, `/pregen start gen radius pregentheworld SQUARE 0 0 66 minecraft:overworld` to pregenerate a square a bit bigger than 128 chunks at a side. This will take multiple minutes (the GUI will show progress). If you want to profile multiple dimensions, do the same for each dimension. Note that the scanning zone currently must be the same in each dimension; you can't profile a smaller zone in dimensions you don't care as much about (well,  unless you manually merge the JSON files from separate scans).
3. After the generation is finished, you can exit the world (and close the client if you want). Open the command line in the folder you placed the executable into and run: 
```
region_scanner.exe --path "<path to your world>" --dims <all the dimensions you want to scan, separated by spaces> --zone -2,2,-2,2
```
Also see options and usage examples in the previous section.

4. Watch the scanning progress. The program currently reports on starting every new region, as well as prints a report for every dimension.

5. After finishing, the program will create (and overwrite if present) a `world-gen.json` file in the `output` folder where you put it. This file goes into the `/config` folder of your Minecraft instance. After reloading the world, your Just Enough Resources should find it and start showing the Ore Generation tabs for every block that was in the scanned area. Filtering is currently not implemented - you can filter the JSON manually if needed.

# Supported formats
## JER
The default export format is a `world-gen.json` file compatible with Just Enough Resources. Some details change with version, but the overall JSON structure is a list of dicts such as this one:
```json
{
"block": "minecraft:jukebox",
"distrib": "39,0.00000762939453125;40,0;41,0;42,0;43,0;44,0;45,0;46,0;47,0;48,0.00000762939453125;",
"silktouch": false,
"dim": "minecraft:overworld"
},
```
where the "distrib" key contains the frequency by level. Notably, the levels are always nonnegative - JER does this by offsetting the level by `64` for 1.18+ worlds, so in the distribution for a 1.18 world, level `5` in the distribution string is actually `y=-59`.
The files generated by RegionScanner have some peculiarities:
- `silktouch` is always `false`, RegionScanner doesn't  support determining how an ore should be mined or what the drops are - this info can't be parsed from the world file alone.
- `distrib` mentions all points from the lowest y-level the ore was found on to the highest, including zero frequencies. For example, above there's only two levels with jukeboxes founds on them, 39 and 48, yet the distribution mentions all levels between too. This is necessary to produce accurate JER plots, since it seems to just connect the points in order without assuming that unmentioned frequencies are zero.

## Tall CSV
Useful if you want to later import the worldgen data into some data science suite. The CSV generated looks like this:
```csv
dim,block,level,freq
minecraft:overworld,minecraft:brick_stairs,19,0.0000019073486328125
```
Notably, unlike the JER format, `level` isn't offset and can be negative in 1.18+ worlds.