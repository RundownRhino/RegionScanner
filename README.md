# RegionScanner
A CLI program to scan Minecraft region files and create Just Enough Resources world-gen.json files from the results. Tested and works on 1.13-1.18 inclusive, but not 1.12 and older.

# Usage:
`region_scanner --dims <DIMENSION_ID> --path <FOLDER> --zone <ZONE>`.

See ``--help` for help. For example:
```
OPTIONS:
    --dims <DIMENSION_ID>...
        The dimension ID in the new format.
        Examples: 'minecraft:overworld', 'minecraft:the_nether',
        'minecraft:the_end','jamd:mining'.
    --path <FOLDER>
        The absolute path to the save folder of the world in question.
        This is the folder the 'region' folder is in.
        Example: 'D:\Games\MultiMC\instances\FTB Presents Direwolf20 1.16
        v.1.4.1\.minecraft\saves\MyTestWorld'
    --zone <FROM_X> <TO_X> <FROM_Z> <TO_Z>
        The zone to scan in every dimension, in regions, in the format of
        'FROM_X,TO_X,FROM_Z,TO_Z' (separated either by commas or spaces).
        For example, '-1,1,-1,1' is a 2x2 square containing regions (-1,-1), (-1,0), (0,-1) and
        (0,0).
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
