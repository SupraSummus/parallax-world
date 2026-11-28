# To do

Known problems, measured or derived, by the part they are in, then open ideas.
Numbers are for the forest at 1280×800, as measured when written.

## Generating

- **worldgen still decides some things with floats.**
  Shapes are kept and tested in fixed point,
  but they are given in floats, and their areas, fills and tubes' radii are f64,
  as is the forest's geometry (sin, cos, pow, exp, atan2, hypot, from `js.rs`)
  and the draws it takes (`Rng::next`, a float).
  They should go to fixed point like the noise,
  the forest building each thing relative to its anchor.
  The fixed-point containment tests cost about 15% at level 0.
- **A box's faces round to the sites.**
  Faces on a plane of a level's sites split its sites ("Faces" in
  `docs/generators.md`); off those planes a face rounds to the sites,
  up to half a site either way: a town's floors at one height all go one way.
  A field's solid spans no longer do: their sites hold the share they cover
  ("Cells hold what they look like", below).
  A sheet midway between two planes is held by both (`Sides`), twice as thick:
  discs all midway showed 0.085 at level 2 against 0.044.
  The city leans on it: half its plan cells are midway at level 2,
  and held by the lower plane alone, they came 0.03 to 0.05 further off.
- **Sheets and rods stop too much seen along them.**
  Grown to a site and opaque ("Levels" in `docs/generators.md`),
  a wall stops every ray across it, as it should, and every ray down it:
  from above, walls a unit thick cover 8 times what they do at level 0
  at levels 3 and 4, posts 4 to 6 times from level 1 (`worldgen/tests/shapes.rs`).
  A site stops rays alike from every direction:
  at levels 2 and 3 the windmills' sails, measured, are seen through,
  rays from all round seeing about half as much of them as at level 0,
  yet from above they take 0.02 to 0.05 of what rays straight down see,
  against 0.002 to 0.02 at level 0, and the tower's plaster 0.03 to 0.04,
  against none.
  Measured far forms count what a thing hides along each axis
  by columns, as rays along an axis meet cubes (`columns` in `generator/forms.rs`).
  On the FCC lattice, an opacity per line of nearest neighbours
  (a share per pair of a voxel's opposite faces, measured as rays meet columns along it,
  each ray taking the share of the line it enters a voxel across)
  was tried and not kept
  ([SupraSummus/parallax-world#205](https://github.com/SupraSummus/parallax-world/pull/205)
  and its branch `claude/measured-by-line`).
  Over random directions it is nearer what a thin thing stops:
  in a cell four fine sites wide,
  0.15 off for a wall a fine site thick, against 0.23 with one share,
  0.06 for a rod, against 0.12.
  But the contract barely moved
  (of 4104 margins, 68 closer and 31 farther, none by more than 0.01),
  as measured houses fill their cells but at their edges;
  and along an axis, as from above, a ray enters by the faces of four lines,
  two of them slanted to a wall across another axis,
  so it sees as much of the wall as with one share, and no six shares do better.
  What would: a voxel holding where in it its content lies,
  between two planes (contours, as in Laine and Karras' sparse voxel octrees),
  so that a wall is thin from every side and need not grow to a site.
  A site's pieces now may stop and glow unlike along each of the 6 directions
  (`VoxelGrid`'s `views`), so far only for a box's or an ellipsoid's sheet
  ("Cells hold what they look like", below).
  Still to do: rods, measured far forms.
- **A coarse site takes a share of a solid a site past it.**
  A box 6 units high on y = 0 gives the site 64 units up a share of 1 in 255 at level 6;
  a wall 20 high, 1 or 2 at levels 5 and 6, though neither reaches that site's voxel.
  A world's heights then need a coarsest site of room over what it holds,
  as the solids' world in `worldgen/tests/shapes.rs` is given.
- **Small things on a ground sink into it at coarse levels.**
  A solid under the sites is drawn at the site nearest it,
  which a few units over a ground on a plane of sites is the ground's,
  full already: what the solid stops is lost.
  From level 4, small blocks, balls and specks on the ground
  keep 0.4 to 0.6 of what they stop from above; lifted clear of it, 0.8.
- **A sheet over nothing, drawn on its own, still breaks the contract.**
  The market hall's timber floor, a child of its own
  ("Levels" in `docs/generators.md`),
  stopped less than level 0 by 0.07 to 0.10 at levels 1, 2 and 4;
  measured with the hall, it darkened the walls over it, so it was taken out.
- **Most surfaces are thinner than `SURFACE`.**
  The valley's snow and landmarks' ground lie as deep as level 4 draws them;
  the fields, towns, the learned town, the moor, the forest and the railway
  still lay theirs half a metre to a metre deep on ground of another kind.
  The depth that keeps a world's levels alike is that of its coarsest checked
  (its entry's `levels`: the forest's is 6, so 16 m),
  so `SURFACE` could be read from it, or `Column` could know it.
- **Some fields still keep their levels by rules of their own** (`sites::spacing`):
  the ponds' reeds, the rails and the city's roads leave detail out by level,
  and the railway's ballast dithers by hand, in shares of iron and timber,
  or, where the rails are as tall as a spacing, bears a medium of them.
  Sizes on spans, as children have,
  and patterns read through `Plan::dithered`
  would leave the spacing to the engine but in the canopy and scenes.
  Found on the way: the town's kerbs (a unit) and the railway's tunnel lining
  are never thickened.
- **Cells hold what they look like** (begun;
  what it means and what the contract makes of it
  are in "Open problems" of `docs/generators.md`).
  Still to do, each a step of its own:
  - Boxes and gables. A box's three axes' slabs taken as if apart (tried)
    are wrong for what is thin along two, a rod or a speck:
    a small lamp's glow grew with the level.
    Each site holding its part's exact shadow along each direction (tried:
    the voxel cut by the planes, each shadow the hull of its corners seen along it)
    kept the solids' contract (`worldgen/tests/shapes.rs`),
    but walls came out pitted at level 0,
    and the village's houses thinner at levels 1 and 2:
    a wall a unit thick on a plane of sites is half of two sites,
    each shown by its own coin, so rays leak through it.
  - Balls and tubes. With them `Sides`' ties and the thin shapes' rules go,
    once a thin wall stays whole across sites shown apart (above).
  - Spans of a medium are held so now (`held` in `drawables.rs`). Left:
    - A see-through kind has one share a site, pieces one a face,
      and a site shows its pieces whole or not at all.
      So it takes its most across the line,
      and a painted one takes the pieces' place through every face alike (`see`).
      Its own share for each face, drawn so by hosts, would end both,
      and the thickening below.
    - One standing free (`Span::medium`: mist, rails) is still spread over two sites:
      in the site of the ground under it, it shows only in the colour of the ground's pieces.
      Likely why the moor's coarse levels show less mist from above than level 0:
      with the mist nearly white, 0.019 apart at level 4,
      so the moor is not checked far away (`far` in `worlds/src/lib.rs`):
      its region was 175 m around the spawn, the stream and the heath's patches.
    - Two measures of one voxel: a medium's length is that of rays along the line (`crossed`),
      what its solids stop, of chords along the faces' directions (`slabs`).
      One for both would move every ground's top.
    - Through the faces along the line, the solids behind a medium are those under it:
      right for a column, not for a line along x or z.
    - A medium as thick as a site lies on its ground through those faces too,
      where it is beside it: a little less is stopped at level 1 (the valley's fields).
  - Hosts: the levels they ask for, the scale a body walks at.
    Seen from near, a level is sharp only where a shape is several sites thick.
  - Hosts show a site held in part whole or not, by a coin of its site (lattice's `shows_whole`),
    so from near it is pitted (a flat ground's top, a site deep),
    and light, which sees its share, lights what shows behind it as if through it.
    Tried instead:
    a coin four times finer (the voxels engine) flickered far off, finer than a pixel;
    drawn as a medium, reliefs smeared the ground and far trees blurred to haze.
- **Layers that don't say where they are are drawn everywhere**
  (`Field::meets`, `Population::stands`):
  the city's buildings, and the towns', landmarks', settlements' and steadings' things.
  What the moor and the learned settlements draw by their shares too:
  they don't bound their bids (`Scalar::new`), so their shares may be anywhere.
  The moor's needs the range of the terrain's height over a rectangle:
  sampled with the valley's slope, a range is either loose or dear.
  Saying so is for speed, not simplicity:
  a promise per layer says again what `heights` says once.
  The railway's piers can't yet: a bridge has no greatest height,
  and `About` is a band around one plan.
  Simpler would be a field of `Column`s on a plan knowing where it is,
  with only what varies by column (water, mist, bridges) declared.
- **`VoxelGrid::get` gives whole voxels alone, `lattice::Grid::get` all hosts are handed**:
  one name, two meanings. Most callers of the first are tests asking for a whole voxel.
- **Whole sites are still put one by one.**
  Under the surface nearly every put is a whole site of the ground's fill,
  each through `put`, its brick looked up, at level 0 its shade hashed.
  A line's run of them put brick by brick would spare most of that.
  Each also has its pick hashed (`picked` in `drawables.rs`, `draw_shape`),
  which only a site put in part reads.
- **Tiles made together are copied twice.**
  `parted` makes one `Grid` of them all (`to_grid`),
  then copies each brick again into its tile's (`Grid::part`).
- **Sites unlike through their faces are the dearest to put.**
  A seventh of the valley's sites are in `views` (`VoxelGrid`),
  which take as much as all the sites' pieces,
  held by sight (`Sights`); each its own six, twice that.
  Half are alike another site's: held once, they took a third less,
  for a grid not to be changed once held. Not kept.
  What a site's faces see, and what is added to them,
  are held each value once (`Faces` in `grid.rs`),
  so a put adds to each class of faces once (`pieces` in `generate.rs`),
  two or three, not six;
  still, at level 4, putting them is over a tenth of the instructions.
  Their glows are kept apart (`glows`), as few glow:
  one record a site would be read and written once, not twice,
  for memory not measured.
- **Plans are made at every column read, whatever their scale.**
  The valley's land, decided at 4 m, makes its seven parts' bids
  at each column of a tile at level 0.
  Bids made every 4 m and interpolated (tried) cost as many instructions:
  most of a bid is the terrain, which the ground makes there anyway;
  level 0 took a tenth less, level 6 a quarter more,
  as things read the land at a point, four samples each.
  Bids that surely leave no share of a square of columns need not be made there,
  nor every bid's bounds each time a layer asks a share over a box (`Land::over`):
  kept by square, both took level 0 about 5 % fewer instructions (tried before `Scalar`),
  little of it the bids, as the moor bids everywhere.
- **Claims cost coarse tiles most.**
  At level 8 a column's ground costs some 24,000 instructions,
  mostly the moor's pools and steadings near it:
  columns 32 m apart each ask their own stretch (`Claims::near`),
  and each new claim reads the ground dozens of times.
  Asked per column without stretches (tried), every level took twice as long.
  Claims reading the ground at their own scale (a band sampled every 8 m)
  would spare about a tenth of the tile, and move pools and steadings.
- **Plans lose columns at coarse levels.**
  A tile there reads more columns than a plan keeps (`SLOTS`):
  claims read the ground where they ask.
  The valley makes its ground 1.3 times a column at levels 4 and 5,
  its nearest town twice at level 8 (the land asks at its dithered points).
  With eight times the slots, each was made once;
  pairs or fours of slots per column cost more than they saved.
  Near columns' slots lie far apart in memory, so most lookups miss the cache:
  on one thread a quarter of the slots took no longer,
  but a host's threads make tiles together; slots keyed in 32 bits saved nothing.
- **Levels share no plan columns.**
  A level reads each site at its cube's middle (`sites::middle`),
  so no two levels read the same columns,
  and a host making every level around the camera makes each one's ground anew.
  Middles chosen to nest would share them,
  but read a coarse site a sixth of it off its middle.

## Checks and cost

- **Cost is counted for more than hosts make.**
  `cost::report` and the baselines count `generate` of what `shown` leaves,
  not `generate_shown`, which skips what is buried under each block (`Floors`):
  they don't see what that saves, nor what finding the floors costs.
  And for boxes four of native's tiles a side, each alone (`cost::tile`),
  where a host makes a tile with those above and below it.
  Nor does it count handing a tile over (`VoxelGrid::to_grid`):
  when that took a power per channel of every site held in part,
  level 4 took twice as long as levels 2 and 6, and only `bench.rs` showed it.
- **Cost counts a shape's tests by its box**,
  though the time is in the sites inside it:
  a slanting twig's box is mostly empty,
  but testing only each row's span of it (tried) took as long.
- **Cost weighs every plan lookup alike** (`NS` in `worldgen/src/cost.rs`),
  though one of a column just read costs a fraction
  of one among claims making their keys,
  and one making the valley's land, many times both:
  the moor's boulders at `Points` counted 15 % less at level 6
  and took as many instructions.
- **The places check finds few things of sparse populations**
  (`worldgen/src/checks/places.rs`):
  it asks COLUMNS cells at most,
  and finds a claimed thing only from those within its reach.
  The valley's landmarks: 118 at eight seeds (not 686 as one per region),
  under ENOUGH a seed, so no seed is tested alone;
  its towns 57, its haystacks 2.
- **Levels past a world's `levels` are not checked.**
  The valley's, the learned town's and the city's stop at 3 or 4,
  where their things break; their fields go on to 6.
  A world could name levels for its fields apart from its things'.
  Not long rays across a wide region, against level 0:
  grazing along a ground, they cross its coarse half-full sites many times over,
  and stop far more than at level 0 (0.67 against 0.5 at level 6), which no alpha keeps.
- **Things' variants of one part, drawing the same kinds, are one stratum**
  (`worldgen::checks::survey`): a ruin and a watchtower, both of stone,
  are measured as one between them.
  Naming a landmark's node by its variant, as towns name their children,
  would part them; or the survey could count which code drew each part.
- **A part's kinds share its places**, so they share its cells
  (`worldgen::checks::cells`): a house's wall and its wood measure the same,
  a line each in the baselines.
  Checked by the part's name (tried), a kind only one variant draws is averaged away:
  the landmarks' worst fell by two thirds.
  Strata by variant (above) come first.
  A cell could count for a kind only where the finer level draws it.
- **A part that only carves is no stratum** (`kinds_of` in `worldgen::checks::survey`),
  though a field that carves is one: a thing's cuts get no region of their own.
- **Rarity is counted two ways** (`Stratum::found`): a population's parts
  in a cube grown until there are enough, a field's columns in all its square;
  and of a field, what only level 0 shows (the valley's mossy stones)
  only at the columns asked there.
  The rarest go first by these counts; places per area would compare them.
- **The survey asks each field at a million columns of 16 km**,
  a quarter of them in 8 km at each of four seeds checked together,
  though the plans they share are made there once.
  Over 4 km, or at columns of level 9, it lost strata (tried):
  the valley's rare ponds and kinds of heath, the moor's stream bed.
  Asked at level 0 at every column too (tried), it took half as long again
  and found no stratum more.
  So what only level 0 shows is found only where coarser kinds' places are kept:
  a kind sparse everywhere would be found little.
  Shared by a hash of the column instead (tried),
  each seed made the coarse plans of all 16 km, so saved half, not three quarters;
  things shared by cell (tried) lost the rare variants
  of the valley's towns and landmarks and the city's buildings.
- **A region's level 0 is made whole, and is a third of its time**
  (`hold_region` in `worldgen/src/checks/contract.rs`),
  to find the surfaces its cells are sampled near:
  rays then see a hundredth of it, and few of its parts are held.
  Made part by part, only where cells are looked at,
  the cells would be those of a few parts:
  near each other, so telling less surely.
- **A stratum joins a region however much it widens it** (`survey::regions`):
  many joins widen theirs by more than the place's own volume.
  But a region's coarse levels cost about the same at any size,
  so a region of its own is not cheaper by volume alone.
- **A narrowed check is held to its world's baseline, measured elsewhere**
  (`CONTRACT_ONLY`): laid out for fewer strata, its regions move,
  and the valley's railway came worse than its baseline by more than `SLACK`.
- **A panic while holding a region doesn't say which seed's.**
  Regions of all seeds share one pool of threads (`check`).
- **The contract's memory is mostly the generator's caches**,
  things measured (`MEASURED` in `worldgen/src/generator/forms.rs`),
  filled as regions move about the world:
  1.5 GB for the valley's four seeds, 1 GB with 64 kept, no slower.
  They are sized for a host's first view.
- **The solids' scenes ask little against the level two finer.**
  In `worldgen/tests/shapes.rs` every solid keeps the contract there
  to the last level checked: a `Look` is a mean over a region's rays,
  so a solid that thins changes it only by its share of the view.
  Scenes the solid fills, the same share for every case, would ask more.

## Worlds

- **A shape can't recolour only what is there.**
  `Op::Paint` draws over empty sites too (a fountain's water is a paint),
  so a shell of another kind round a hollow (limestone round a cave,
  moss on a rock, snow on a roof) stands in the air where the shell leaves the ground.
  An op that paints only sites already held would make one;
  fields of spans paint a ground's top so (`Column::surface`), shapes cannot.
- **Things fill coarse sites a carve left in part.**
  The caves' rubble, filling the bowl of a chamber dug by a field,
  stops a sixth less at level 2 and over than two levels finer
  (`worlds/data/contract/valley.txt`), as boulders half sunk in the ground do:
  a fill takes only what is empty, and a coarse site at a hollow's wall is neither.
- **A part can't stand in a world twice.**
  Its layers are known by their names alone (`Layer::name`):
  two forests' trees would be one layer to `alone`, the survey and cost,
  and a field's coins are seeded by its name (`draw_field`).
  And its kinds' ids are its place in a table every world concatenates
  in one order (`kinds!`'s `base`); named, the world would number them.
- **A land in space is read at every site of a box it is not whole in.**
  The mine's rock is the cheese painted where it owns it (`rock` in `mine.rs`),
  each site deciding the shares anew, as nothing keeps them;
  asked in cubes 16 sites wide (`SITES` in `drawables.rs`),
  only those its lumps' edges cross are read.
  From level 6 the lumps are a few sites wide and every cube is crossed:
  a tile reads a million sites.
  Spans along an axis would find the edges on each line
  (the lumps rise or fall monotonically in each of their cells),
  or the shares could be read dithered as a ground's are.
- **A part owns a column's things, cover and air at once.**
  So a landmark's clearing, kept for itself to keep trees off,
  cut the moor's heath and mist away.
  The moor now lays its heath over the landmarks and its mist over them
  and the railway, by its bid without them (`moor::Alone`),
  but a landmark among fields or dense forest paints its own grass.
  Land could share cover and air apart from things,
  a part bidding for its things only where it builds.
- **Worlds share tables of kinds**, so the contract only prints
  the kinds nothing surveyed draws, and fails on silent layers alone:
  the moor's is the valley's, the learned town's the first of it.
  With only the kinds it draws, a world could fail on a kind
  drawn only far from spawn, or never.
- **The worlds but the cheese share one crate**, as they share pieces:
  the forest's ground and kinds, the valley's parts.
  A world in a crate of its own, using others' pieces as crates,
  runs into the two items above and "The layout has more to sort".
- **The railway's track is too bright seen low from level 2.**
  At level 0 its sleepers and rails hide the ballast from a ray coming in low;
  at level 1 a medium of them over it does (`railway.rs`, as `Column::medium`);
  from level 2, shorter than a spacing, they are its top's dithered shares,
  as seen from above, and the grey ballast shows from every side.
  A field giving its sites a kind per face, as a thing's far form does
  (`Look`'s faces, measured in `generator/forms.rs`),
  would keep the ballast's shares on the faces looking up
  and the sleepers and rails on the others.
- **Generating the forest's grids took 3–8 s of a thread's time per level**
  (in WebAssembly, in Node, when the web drew grids; the TS forest took twice that),
  most at levels 0 to 4 (a moved grid, a seventh of that).
  In the TS forest it was mostly the ground's noise,
  which the terrain and the canopy compute about 14 times per column
  (with its neighbours, for slopes),
  and at levels 0 and 1 the undergrowth's many small shapes;
  worldgen's, counted ("Cost" in `docs/checks.md`),
  goes mostly to the trees at levels 0 to 2
  and to the canopy from level 4 (`Rooms::owner` making the trees near).
- **Most of the cheese's tiles are over the cost budget**
  (`Known::OverBudget`), at the spawn and in the mine:
  cubes hold twice the sites FCC's did in a tile,
  so at levels 0 to 5 the cheese costs up to 1.7 times its budget;
  at levels 6 and 7:
  at level 7 a tile holds a few hundred thousand holes,
  each a shape, most of them a site or two
  ("Far forms for what fills space", below);
  at level 6, the crystals' far form, a ball about each cavern,
  and the grottoes each making the holes around them to place their stairs.
  The mine's lumps, where none of that is asked, brought the rest under.
  Timed as hosts make them (`bench.rs`), most of level 7 is not drawing
  but asking a tile's holes and far crystals, once (`Contents`):
  the land's shares at each cavern's centre (the mine's bid),
  and each made a shape, though most are a site or two.
  The estimate counts drawing only ("Cost is counted for more than hosts make").
  Estimated, not timed: the cheese's body, all of a kind, is filled site by site
  uncounted, a good part of a coarse tile's time.
- **The learned town's rules are from the old towns.**
  `worlds/data/town-grammar.bin` was learned when the valley's towns
  were all market towns alike, so lab's `export -- --check` says it differs.
  Learned again from today's, mostly hamlets and villages,
  it draws far less town: 235 houses in `learned.rs`'s test, not over 600.
  Learn it from the market towns alone, or let the learned town be sparser.
- **The forest's canopy measures crowded trees in sample stands.**
  From above it reads each column's owner (`Rooms`),
  but what the trees standing in a column hide from the side
  comes from stands at four densities, read by the trees' chance.
  A crown alone measured by offset and slab (`Shadows`),
  read for each column's owner, would leave rooms the one rule.
- **The forest on steep slopes breaks the contract at level 4.**
  A region on a wooded mountainside of the valley,
  where the trees thin out to the treeline,
  came 0.10 to 0.15 apart at level 4, in materials hit and from above:
  leaves in place of trunks and of the slope's scree and rock.
- **The cheese's mine keeps to its lumps' lattice** (`Known::Prefers`).
  Its lumps are value noise on a lattice 96 m wide (`LUMP` in `mine.rs`),
  eased between its points, so most extreme at them:
  the mine holds the same places of every period, at every seed,
  and the grottoes, in the cheese it leaves, bunch between
  (`even::cheese`, along y).
  Noise not eased to its lattice (gradient or simplex), or turned,
  would spread it.
- **The city and the cheese's mine line up with the lattice.**
  Their entries list them (`Known::Turned`).
  The valley's towns lay out streets and blocks along x and z,
  and build in frames of quarter turns (`frame.rs`):
  one town, the same at every seed, drew a four-fold harmonic of 10 %;
  the town nearest the start, another at each seed, draws none the check tells.
  The city is boxes on a plan, all along the axes: 4 %.
  The cheese's mine digs galleries along x and z: 2 to 3 %,
  around it and anywhere, where its lumps are a quarter of the space.
  The learned town's settlements are such boxes too (2.5 %),
  but their streets' own direction (2 %) hides them from the check.
  Square noise (`ValueNoise`, `Fbm`) draws under 1 %, too little to tell.
  On cubes the cheese's holes around its spawn line up too, as they did not on FCC:
  −1.5 %, pairs along the diagonals parting less often than along the axes.
  `mountains.rs` turns its noise by 37° (`turned`),
  which leaves its four-fold harmonic mostly on the diagonals;
  22.6°, (12, 5)/13, would leave none there.
- **Measured houses show their walls from above at level 6**,
  which the contract does not check, and at level 4 in a region of houses alone
  (seed 2: plaster 0.19 against 0.01 at level 0, 0.106 apart;
  within the contract since the oaks among them are round, likely by chance).
  Around the valley's first town's centre (seed 42),
  level 6 shows 46 % houses from above against 25 % at level 0,
  and 16 % roofs against 24 %: the rest is walls, timber and stone
  (0.31 off; 0.18 measured from level 1, 0.22 from level 3).
  Measured from level 4, a house is a few fine sites
  ("Levels" in `docs/generators.md`):
  its eaves, thinner than one, leave its walls' tops to show,
  and its gable ends (`gable_ends` in `town.rs`), grown to one,
  paint its roof's ends.
  And a cell's share counts rays the cells behind it stop too,
  so a thing keeps more cells than it stops rays.
- **The valley is still laid out along x, across spines by z.**
  The path is a curve (`worlds/src/path.rs`),
  but the ground's walls and dip, the fields' plots
  and the landmarks measure across its spine (`Spine`),
  and the ranges across the valley's course (`worlds/src/valley/course.rs`),
  so the valley winds, but never turns back on itself.
  A spine along a curve, across and along it, would let it,
  but a curve's distance folds past a bend's radius
  (16 m at the tightest, where the path bends onto a street),
  which would crease the walls and the ranges,
  and costs a search far from the curve;
  what is far would read a coarser band of the curve, its mean.
  The moor's stream (`stream_distance` in `moor.rs`) is measured along x too;
  from a curve its banks would move where it bends, so it is left as it is.
- **A path's bend is promised by hand** (`BEND` in `worlds/src/path.rs`),
  for each world's route, though its slope is measured:
  `worldgen::curve` takes a wander per level, fixed for the whole curve.
  Corridors measured piece by piece would let a curve keep its own.
- **A station stands along the axes, its track may not.**
  Where the valley bends at a town,
  the track runs up to 30° off z past a building along it
  (`Station::frame` in `worlds/src/valley/railway.rs`).

## Light

- **Light is a prototype, in look's pictures and native's computed lighting**
  (`light/`).
  [docs/light.md](light.md) says what it gets wrong against its reference,
  what was tried and what it costs.
  Where it is to go, a step at a time, each measured on the reference's scenes:
  sites that hold what they stop along each direction, boxes' too
  ("Cells hold what they look like", above);
  then coarser levels taking more directions (in Ideas).
- **Thin walls kept the sky out of sheds along the lattice; nothing else was measured since**
  ([light.md](light.md), "Thin walls").
  Not walls slanted to it, nor the valley's railway tunnel,
  which was lit hundreds of times too bright before.
  A shed of cuboids turned about y would say the first,
  as the reference's roof is turned.
- **Light settles slowly in what reflects most of it, spread wide.**
  There a site takes tens of rounds,
  each reading the coarsest level a line's length further round;
  rounds are capped (`ROUNDS`), so light that bounces more is lost.
  A coarser level cannot take over the rest at the coarsest.
- **A site's rounds find each site they read anew** (`Light::coming`):
  its tile and its cube, round after round,
  as much work as reading its round.
  Held from the site that reads it, they would hold each other:
  sites reflect on each other, so the Lights would never let them go.
- **Two ways to find one light**: Light's rounds, and `Settling` (relax.rs),
  which keeps it settled as it changes ([light.md](light.md), "Settling as it changes").
  The rounds could go once `Settling` forgets what is far,
  is shared by the world's threads (a neighbour's light a moment old still settles),
  keeps its light packed, and finds what is near first.
  Till then, settled, native lights on one thread,
  and what it holds, several kilobytes a site, grows as the camera goes.
- **Light forgets by how many sites it began, not by what they hold** (`KEPT_SITES`),
  each level by its own, the same number for each,
  though each byte a site keeps costs time too,
  and below what a view reads at once lighting slows several times over
  ([light.md](light.md), "In native, and what it costs"):
  counted by bytes, smaller sites would be kept longer.
  The levels read unlike: in the forest's first view, with fewer kept,
  levels 1 and 5 were begun four times over, the rest about once.
  The largest misses are a reflecting site's rounds,
  a pointer from its cube (its `Site`).
  Each level forgetting on its own, its tiles could be kept apart too
  (`Kept`, one map a level), not filtered by level when it forgets.

## Native

- **The flat engine's pictures fall by the order chunks are drawn in**
  (architecture.md, section 3).
  Where neighbours' pictures overlap at a depth, the one drawn last shows,
  and that order is the order chunks came in and found room in the buffers:
  the same view drawn twice differs in about a pixel in forty
  (`few_differ` in `native/src/engine/chunks.rs`).
  A picture's own bricks could win over a neighbour's enlarged edge.
- **A surface along a flat face's axis is smeared.**
  Seen along an edge face, ground and walls along the axis
  are a row or two of columns whose pillars run through the cell,
  and the picture's plane cannot follow them:
  in the city, looking along a street, the pavement at the sides of the screen
  bends, and ledges show a voxel or two off.
  Near the eye a pixel or two of sky shows between pictures on a slope.
- **Hiding cells on the CPU saves the flat engine little**
  (`Hiding` in `renderer.rs`):
  of the forest from its spawn it hid a few dozen of fourteen thousand chunks,
  and the GPU found a third of those it hid showing after all.

- **The native frontend is missing pieces.**
  Its camera is f64, not fixed point,
  and the GPU gets the eye in f32:
  far from the origin it loses its fraction.
  Walking in the window for minutes, a frame sometimes takes seconds
  until the driver resets the GPU (seen on Intel UHD with Mesa);
  the window then makes its device again (`window.rs`).
  Why is open: the same views and paths, drawn headless, take milliseconds.
- **Native's world lists all it is asked for every frame** (`World::update`),
  thousands of tiles when flying fast, and frees what comes far:
  a tile is made with all those above and below it (`made_with`),
  in the valley kilometres of air, hundreds of tiles a frame.
  Leaving the queue alone while asked the same took exceptions
  (a tile dropped since, bricks to light) for milliseconds no frame was short of.
- **Native's threads take more work together than one alone**:
  completing the valley's first view, lit, four take about a third more than one.
  Little of it is work done twice, now that they share the world
  ("Bands" in [generators.md](generators.md)):
  a generator each, they made a fifth more of the plans' columns
  and a sixth more measured things;
  sharing spared four threads a fifteenth of their work
  and each the tens of MB its plans held,
  for a thirtieth more of one thread's alone.
  The rest is every function alike taking longer on shared cores
  (four virtual ones, a software GPU beside them): not looked into.
  A lock on each of a plan's slots cost one thread a tenth of generating,
  as much as four gained: so `Kept` in `worldgen/src/recent.rs` reads without one, unsafely,
  for plans and points alike.
  Tiles side by side still make again the things reaching both;
  those one above another are made together (`made_with`).
- **A `Large` never forgets** (`ground/src/plan.rs`): it holds each piece of a curve for good,
  read where it is, and panics past `LARGE` of them,
  thousands of kilometres of the valley.
  Forgetting would take a count of readers at every read, at every column.
- **Walking, native keeps more than a core busy**
  (the valley, `--stroll 20 --walk`, outside the GPU driver),
  measured when it drew slices: new tiles took about a third.
- **Native's voxels, lit, are lit slower than the eye moves**
  (the valley, `--light 1,1,0 --stroll 8 --speed 100 --by-work`, and walking):
  four threads, the cores to themselves,
  owe the view hundreds of bricks all the while,
  so each level shows lit less far than it reads, the next coarser past there.
  Much of their work is then generating, the tiles the light reads
  ([light.md](light.md), "In native, and what it costs").
  Its tiles come in time, made before the bricks are lit.
- **Native's media** (`KindDef::medium`), added up along each ray
  (architecture.md, section 2): the moor's mist, the fog's (`world=fog`)
  and the valley's sea (`mountains.rs`).
  - The sea's top is a medium of its own, so that where it ends shows;
    it mirrors no sky.
    A kind's shine, kept by sites as glow is,
    would let a ray take the sky's colour where shine changes along it.
    Ponds, the moor's water and wells are still whole voxels.
  - Lit (computed), the sea from above is darker at the finer levels,
    a ring where they meet (`seed=12345&world=valley&camera=-51216,3251,-9490&look=-473,-90`
    with `--light 3.5,1,0`); shaded, it is not.
    Left for now. Likely the light on a thick site:
    a coarse site of water is lit whole by the light on its faces,
    where the fine sites it stands for are each lit through those above.
    No check sees light across levels: the contract's rays have a white light all round.
  - From inside the sea, the voxels engine shows bright dots along two diagonals
    of the picture (`seed=42&world=valley&camera=-36000,-336,40000&look=90,0`),
    as it did before: not looked into.
  - At the levels whose sites are a few pixels wide, the sea shows rings (the valley, from 600 m up):
    a ray for every 4 × 4 pixels takes its medium from a lattice of see-through sites as fine.
  - A site's pieces, where shown, hide its medium and what is behind:
    right on average, but a coarse site of a crown in mist
    is a whole voxel or mist, never the crown's grain seen through mist.
- **Native's voxels show what rays ask for a frame or two late**
  (architecture.md, section 2):
  turning, what comes into view shows coarser first,
  as every ray past the window's edge does,
  till its tiles are made and its bricks placed, and lit.
  Every tile around the eye made beforehand spared little of that
  (lit and turning, an eighth fewer bricks asked for a frame; shaded, none),
  for many times the tiles: not kept.
  The light takes twice the bricks' memory, so the pool holds a third as many.
- **Native's voxels, shaded, read each level as far as lit**,
  now that their pool holds only the bricks asked for:
  twice as far as when it held every brick around the eye,
  finer far off, and more steps a ray, so more of the GPU's time.
  Seen on a software GPU only; how far is worth it on a real one is open.
- **Native's voxels ask for a tile before any light, however far it is**
  (`Voxels::frame`):
  tiles and bricks in one order, nearest first, was tried,
  and rays took a coarser level at half as many bricks again,
  as a tile's bricks are asked for only once it is made.
  And a window grows when all its ball would gain fits (`Windows::sync`),
  counting, shaded, every brick of it, and lit, only those lit.
- **Native, lit, holds more than twice what it does shaded**,
  and more as the eye moves:
  a tile's sites (`Shared`) are kept beside the grid drawn of them,
  though a brick lit needs them no more, till one beside it is lit;
  and the light of the tiles read lately, a few hundred MB (`KEPT_SITES`)
  ([light.md](light.md), "In native, and what it costs").
  Keeping only the sites read last was tried, twice:
  lighting slows, by tiles made again.
- **Native makes more tiles than rays ask for.**
  A tile is made with those above and below it (`made_with`),
  and rays ask for few of them:
  those far from the eye are made and dropped at once,
  the rest held, most of them empty, each with its indexes (`Tile::new`).
  A host's tiles as they are made, a box each, would hold and index them once;
  `lattice`'s `Tiles` and the windows take cubes.
- **A window looks through all its level's tiles at every step** (`retarget`),
  for those gone and those new,
  though a level is handed over in blocks shared with the last (lattice's `Blocks`):
  on the windows' thread, by the tiles held, not by what changed.
- **A world lit only later makes its tiles twice.**
  Shaded, a tile's sites are not kept (`Queue`'s `lit`):
  the light, turned on in the panel, makes them again.
  One store of tiles for hosts, lit or not, would make them once,
  for what the sites of shaded tiles take;
  it would not be the light's then (`Shared` in `light/`).
- **How far past an engine's reach tiles are kept is chosen**
  (`KEPT` in `native/src/world.rs`),
  not derived from how far the light reads (`top_steps` in `light/`, and each level the next).
- **Native's voxels wait on memory** (`march` in `voxels.wgsl`):
  most of a frame is its samples' rays, some 50 steps each,
  and a step costs its reads one after another (a brick's slot, then its site),
  not its arithmetic.
  On Intel's GPU (Mesa) the shader runs 8 pixels a thread, not 16: too many registers.
  Tried on the FCC lattice, and no better: stepping by increments;
  cells of 128·FCC (a window's torus is 4 of them a side,
  so cells a window apart share a count: the sky the ground's);
  each empty voxel saying how clear around it is
  (a third fewer voxel steps, most turned into jumps: 2%, tiles twice as slow to make);
  a rhombic dodecahedron about what a window holds, for its box or with it
  (its faces slant across ground that lies flat in the lattice's axes);
  a box of the voxels of each brick, or of each block of them,
  for the cells or beside them (a third slower, and a tenth).
- **Native's voxels while the eye moves** (`Windows::sync`).
  - A step moves one window, each in turn,
    so level 0's ball moves a brick every ninth step:
    flying faster, it lags, read less far, till it starts anew.
    Stepping first the window that holds least of what it should around the eye
    (`Ball::about` against its radius), not each in turn, may do better.
  - Walking, frames come every 33 ms:
    CPU and GPU together take a little more than the 16.7 ms
    the window paces frames by.
  - A ball's step counts the bricks it gains and loses in the cells around them (`set`),
    by memory, not work: some 10 ms while the world's threads work.
    On the windows' thread that is no frame's time,
    but what a step changed comes a frame late for each.
    In a browser a frame still does the step itself.
  - A frame copies the whole table and counts of each window a step changed.
  - A painter made anew (the first frame, the light switched) takes a frame of some 50 ms.
  - The world keeps tiles, and lit, their light, by distance alone:
    flying lit at a render distance of 40000, some 8 GB.
  - A site's faces see three sights beside its own look (`SIGHTS`):
    with more, far off at coarse levels, the rest see its own.
    worldgen holds sights already (`Sights` in `grid.rs`),
    `lattice`'s `Grid` a look a face, which the engine packs again:
    a grid holding sights would spare that, and say how many there may be.
  - Shots and tests wait for the windows a step at a time (`STEP` tiles),
    a whole picture drawn each, so the voxels' GPU tests take twice as long.

## Browsers

- **A browser's threads gain less than native's.**
  In headless Chromium on four cores,
  the valley's ground about a walker (six times its reach)
  was made in 3.5 s on one thread, 2.2 on two and 2.0 on three,
  and in 3.8 on the main thread alone, as before there were threads.
  Not looked into: they share std's allocator, whose lock spins in WebAssembly,
  and the GPU, in software, takes cores too.
- **A device lost in a browser can leave the page blank.**
  The window makes its GPU anew (`native/src/window.rs`),
  but Chromium may give that page no adapter after a GPU crash,
  and the window panics on none;
  the page asks for one only at the start.
- **The page is not tested.**
  Headless Chromium shows nothing of a WebGPU canvas on a software GPU;
  a shot would read its frames back from a texture of its own.
- **Node is kept for Vite alone**,
  which serves the page while developing
  and names its files by their hashes,
  so that a browser never pairs an old module with new bindings.
  `web/bindgen/` could write the page's directory itself, hashes too,
  and the scripts would be cargo's.
- Keep the view in the address bar as the camera moves,
  so a view can be copied (the page reads it already).

## Tools and CI

- **Tests take most of CI's time.**
  Most of it is the worlds' contract, bound by the CPU, in jobs of its own;
  the valley's, alone, is CI's longest job.
  Its threads end together, so only less work would shorten it,
  or its seeds on several runners (`--nocapture` shows each part's time).
  A seed's regions take longest, then its survey.
  If CI is still too slow, the contract at seeds but `SEED`
  (`worldgen/src/entry.rs`) could run only on main.
- **LLVM 21 compiles AVX-512 code wrong (lavapipe).**
  It may rematerialise a vector of all ones into a register past 15
  in an encoding that cannot name one: the ones land in another register,
  and that one keeps what it held
  ([llvm/llvm-project#169009](https://github.com/llvm/llvm-project/pull/169009),
  fixed in LLVM 22).
  Where it strikes moves with the CPU LLVM tunes for and the shader's shape.
  Ubuntu 26.04's Mesa 26.0.8 on LLVM 21.1.8, tuned for Zen 4
  (CI's EPYC 9V74 runners), reads ymm29 in `voxels.wgsl`'s march unwritten,
  and most rays run out of steps.
  So CI's `native-gpu` runs without AVX-512, until its Mesa is on LLVM 22.
  To reproduce on a CPU with Zen 4's AVX-512:
  that Mesa (in a chroot will do),
  `llvm::sys::getHostCPUName` made to return `znver4` by `LD_PRELOAD`,
  and `MESA_SHADER_CACHE_DISABLE=true`,
  else lavapipe reuses what it compiled for another CPU.
- **The layout has more to sort.**
  - `worldgen` holds more than generating:
    `checks/` and `cost.rs`, which only tests and examples use.
    A crate of their own, as `light/` is, would say so,
    but they read the generator's private parts (`whole`, `checking_skipped`),
    and a world's `Entry` holds the contract's `Accuracy`.
  - `worlds/src/forest.rs` is still a world, the ground others build on
    (`Relief`, `Forest`) and the table of kinds they draw with (`K`);
    the valley's parts (`worlds/src/valley/`) use each other in circles.
    The kinds and the ground apart
    come before "The worlds but the cheese share one crate".
  - `worlds/data/cost/valley.txt` is the repo's largest text file,
    grows with every place the valley names,
    and changes with most commits to the worlds;
    a baseline per place and level would do for a test with a quarter's slack.
  - `docs/generators.md` holds the worlds beside the engine ("Composing").

## Mechanics

- **It is slow.** A forest tree of 3000 cells, 7000 nodes, at level 0
  simulates about four times slower than it falls,
  and over ten times once it lands, on one thread;
  the matrix products could share the cores.
- **What a cut wakes is a rule of thumb** (`Around` in `native/src/mechanics.rs`):
  what is over it, near it, as much as `BUDGET` cells.
  A branch cut at its foot whose tip droops lower stays, held by its tip,
  and what is farther stays asleep, and may hold up what should fall.
  What falls depends on what holds it up, which is not local;
  the coarse levels of one solve over all about the cut could find it.
- **What is hit wakes only where it is borne on hard**:
  a stone a log lands on stays put,
  and a site woken is welded to its neighbours asleep,
  so a tree hit stands while they sleep.
- **Loose matter is carried, not simulated**,
  may be left behind where a cell hops into its site,
  and is crushed where it is carried onto what bears.
- **A hop that is blocked waits**, and the matter shows a site behind
  where it is, till it is free.
- **A step is solved to a share of its momentum** (`TOLERANCE`),
  so what falls fast is left strained by the solve's error, and may break.
  Solved to a share of what changes instead,
  it stalls among loose pieces in contact: the V-cycles fit them badly.
  And what is still takes as many rounds as what moves,
  of a momentum that is nearly nothing;
  solved no finer than a tenth of `SLOW`,
  what starts to topple from rest never starts.
- **What falls asleep is written back at its sites**,
  up to `HOP` of a cell from where it lay,
  so what stays awake beside it may overlap it at once,
  and its contacts push a hundredfold past what crushes them,
  more than `CONTACT_ROUNDS` settle:
  a forest tree that has landed and lain still
  can blow apart as half of it falls asleep.
- **A tree of the forest stands on what bears nothing**:
  its floor is loose matter metres deep, so a tree cut through falls through it
  and lands far lower, its crown's cells heavy with the leaves they carry;
  landed, its steps take several times the rounds its fall's do.
  `native/src/mechanics/tests.rs` follows its fall, not its landing.
- **A step's contacts settle in `CONTACT_ROUNDS` solves at most.**
  In the few steps they do not,
  a contact let go after the last solve has still held for the step, as glue,
  and one let slide has held past friction.
- **A cube's shear at its corners is partly the cube's own**
  (a trilinear element locks in bending):
  a weld judges shear by the mean over its face, and so not along an edge.
- **An edit is lit again a tile around it**;
  a shadow cast farther stays till the world is lit anew.
- **All that bears is wood.** Worlds give no materials.
- **Worlds give no bodies**: generated matter holds on across every face
  (`worldgen/src/edits.rs`), so a pile of stones a world draws is one stone.
  A thing's sites could hold on to each other and not to other things,
  sand to nothing.
- **A thin thing that lands turned loses what holds it together**:
  a rod a cell thick lying at 45° lies as a staircase,
  its cells meeting at edges, which hold on to nothing,
  so a second fall takes it apart.
- **A cell partly full bends as a full one** of its stiffness ([mechanics.md](mechanics.md)).
  Its bending from where in its cube what bears lies would make levels agree.

## Ideas

- Mechanics at several levels at once (`mechanics/`):
  the finer near the cut or the eye, the coarser beyond,
  a coarser cell taking its parts' stiffness, as the solver's levels already do.
- Mechanics that wakes by itself: whatever is edited, not only cut.
- Mechanics by local rules (`local/`, [mechanics.md](mechanics.md)) in place of `mechanics/`,
  once it is fast enough: it has nothing written back, locking or co-rotated,
  but its rounds are as short as sound takes to cross a cell.
  On threads it is still tens of times slower.
  Its time is the springs' arithmetic, and a round a fifth longer is unstable,
  so what is left is fewer rounds, or cheaper ones, each rule kept local:
  implicit steps relaxed round by round over the levels, as light is settled,
  so that what spans more than a few cells is carried by the levels, not by sound;
  a coarser level, sixteen times less work a level up;
  a compute shader, a cell reading only its neighbours, as the threads do.
  Its contacts too: cubes across faces, held by a spring till they slip,
  would stand a face on a face, and hold it without creeping
  ([mechanics.md](mechanics.md), "By local rules").
  Then every spring would sit on a face, and what falls asleep could keep
  each spring's strain instead of where its matter lies (`still`):
  asleep at its site's middle, it would bear as it did, and wake as it slept.
- Liquids on the lattice, by lattice Boltzmann:
  each site's water streamed to its neighbours and mixed where it is, all local,
  with a free surface; its steps need no real speed of sound, so are long.
  Cells of `local/` would be what it flows around.
- A cell's stiffness from the continuum (`statics/`).
  A cell's 15 beams with a section fitted (`SECTION`)
  could be a 36×36 stiffness of its faces, from E and ν,
  coarser levels condensed from finer ones exactly, nothing fitted;
  what it bears read from its stress, against its strengths.
- A population's N (`Population::cell`) does four things now its points are given:
  the spacing of its default points (`make`), its default `size` (2N),
  how far off a ground it may stand (`on`), and the scale checks look for it at.
  Some are left from when its cells were asked: the towns' is 256 m, 2 km apart.
  Each said where it is meant would leave N to the default points.
- `Claims` propose their spots by a Poisson process of their own, as `Points` do.
- Parts claiming things try a few places round each spot, each part in a loop of its own
  (landmarks, caves, the coast, the badlands); `Claims` could try them itself.
  On `Points` they would need cells a power of two wide, their chance the rest.
- Where a population stands could be a scalar too (`Population::within`):
  on a ground, N less how far above or below it a point is (`on`),
  bounded over a box by the plan's range (`Sloped::range`).
  So could where a field draws nothing (`meets`, `about`):
  one rule for where any layer may be, but `Meets::All`.

- The forest's and the learned town's paths are still sines about x = 0
  (`Relief::path_x` in `worlds/src/forest.rs`);
  a route along their floor (`worlds/src/valley/course.rs`) would serve them too.

- The valley's geography from water, not along one line:
  rivers that drain the ranges to the sea, lakes in basins,
  and the valley one of them;
  then paths as curves between towns (`curve.rs`), not `path_x(z)`,
  which towns, the railway, landmarks and the moor all read.
  Now the ranges, the sea and the upland wind with the valley's course,
  along z for ever.
- More scenes (`worlds/src/scene.rs`): Bistro, San Miguel.
  Not tried in them: walking, on walls a site thick.
  Lit (`--light 1,1,0`), Sponza's atrium is all but black; not looked into.
- Light along more directions at coarser levels, as radiance cascades do:
  near, light wants fine places and few directions; far, coarse places and many.
  Light keeps 18 at every level, too few far off (blurred shadows).
- The eye at night: colours fading and bluer (Purkinje's shift),
  so that a night shown brighter still looks like night.
- WGSL formatted in `pnpm lint`, as rustfmt does the Rust.
  wgslfmt 0.1 (wgsl-analyzer's) pads compound assignments (`x   += 1`)
  and joins long expressions into one line; try a later one.
- Assume no ground.
  Hosts keep an up (`world::UP`, +y): bodies fall along it, the sky and the sun are over it.
  A world could name its own, which hosts would then follow.
  But the engine still takes a ground where a world has no say in it:
  - `shown` trims a box along one axis alone, and `Floors` find what is buried
    from the bottom up (`on_rock` keeps it).
    Halved along every axis it would not pay:
    asked brick by brick, `whole` cost many times generating the tile.
    Blocks of columns finer than `FLOOR` would take a ground's range
    from columns between the level's own, each made for it.
  - Places and the survey sample columns of cells along up (`PER_COLUMN`);
    `directions` measures the four-fold harmonic about up alone.
  - Light takes its sky from above y (`SKY`, `SUN` in `light/`),
    and ends a line as far as the world's heights are high (`top_steps`).
    A world's generator could say where light comes into it from outside,
    at which sites of which levels:
    a sky over a ground would be such a world's, not the engine's.
- Far forms for what fills space: the cheese's holes are carved
  while wider than the sites are apart (past level 8 it is all buried),
  and its pillars and stairs stop at level 3
  (its crystals go on as their far form, glowing pieces on the caverns' walls).
  Holes as things that cut the world cost far more: each a thing, its cells asked.
  Holes far smaller than the sites are rightly left out:
  a ray across a site of cheese with small holes still meets cheese,
  so they are no medium carved, as small solids are one filled.
- Far forms from counts, by the engine, for every population at `Points`:
  where things are smaller than the sites,
  each site stops what the things its cell holds do,
  each mark's shadow along each direction measured once.
  It would leave `size`, `far` and hand-made `Shadows` to the engine,
  and keep boulders, logs and plants seen far off, which now vanish past their `size`.
  For correctness and a simpler contract, not speed:
  what vanishes costs nothing now.
  Things about a site wide are where it is least sure:
  drawn, their shadow lies where they are; counted, spread over their cell.
- Lay out towns at any angle?
  Walls along the lattice's axes look crisp;
  at other angles they jag.
  But every town and the city runs north–south and east–west,
  which at the scale of a layout reads as a grid.
  A middle way: streets and plots at any angle, the buildings on the axes.
  The buildings' walls would still show to `directions.rs`.
- Keep the fit of `cost::NS` in the repo, to refit it on a quiet machine.
- Measure the seams between faces against ray-cast perspective.
- Show render distance in metres:
  they are lattice units, and the forest's unit is 0.125 m.
- Show the forest to 22 km again: one level more (`GRID_LEVELS`).
- Learn a world from examples instead of the split grammar.
  Two models in lab draw any box alone, flat.
  The masked model (`lab/src/masked.rs`)
  learns the statistics and new layouts,
  but breaks long thin things (lanes, tunnels) into pieces.
  Synthesis by coordinates (`lab/src/exemplar.rs`)
  copies pieces of the examples, so it keeps lanes whole,
  but cuts houses where pieces of two towns meet.
  The critic (`lab/src/critic.rs`) says what tells them from real maps
  and what they copy;
  profiles (`lab/src/profile.rs`) how their parts are laid out.
  Drawn to a plan learned from real places (`lab/src/plan.rs`),
  shares with main roads and rivers as lines,
  both blend where the plan says, and keep the lines whole;
  but the streets they draw themselves fall into pieces
  (the masked model's, to a few stray draws: `lab/src/lib.rs`),
  and both draw paving in specks.
  Worth trying: a draw that settles neighbouring cells together,
  not each alone (cells drawn in an order that keeps neighbours apart early);
  a plan composed for a world worth walking through
  (landmarks, views, contrasts), not an average one.
- The critic judges flat maps only.

Composing generators (section "Composing" of `docs/generators.md`), untried:

- More populations at `Points`, their far forms from what stands:
  the forest's boulders, logs and plants, the valley's haystacks.
  Moved alone they cost as much:
  their chances, read at every leaf's site, read the land
  as their refused anchors read the ground.
- Bid per quantity: a land per channel (use, ground, wetness),
  each part bidding in those it cares about, as the towns' terrace needs.
- Shares within shares: the forest's species are a land of its own by hand
  (`species_chances` in `forest.rs`);
  `land::shares` of their bids, times the forest's share, would make it one.
- Edges between parts as a product of shares (`Scalar::times`):
  scrub where the forest meets the fields, reeds round a pond,
  bounded over a box like any scalar.
- Place the houses as things of their own, from the plan of lots,
  not as children of one thing per town:
  a box in a town now draws the whole town's carves aside, in a buffer
  as large as the box, where each house's would do.
  Counted as `Points` by kind, they could be drawn far away from their counts,
  not each drawn whole to be measured.
- Let the framework filter plans of numbers finer than the level's sites
  (`Plan::sampled` at the spacing, as the ground is read: `Sloped::at`),
  since plans declare their scale:
  coarse levels would skip fine bands' cost as they skip their things.
  Plans of categories are read dithered already ("Plans" in "Levels").
- More of the railway (`worlds/src/valley/railway.rs`):
  a graph of lines between stations, made locally, and branch lines;
  catenary poles; a road from each station to its town;
  water in the route's cost, once the ground the line reads has any.
  Curves still lack arc length from an end (`Near::along` counts segments):
  the line's sleepers and piers stand by its segments along z.
  Its grade turns at once where it tops a rise or bottoms out,
  often from the steepest up to the steepest down:
  vertical curves would round those, as smoothing rounds its bends.
  What carries the track is decided by thresholds, apart from what it costs,
  and runs shorter than `MIN_RUN` are cut or filled after,
  however deep that leaves the earth.
  From prices alone (earth by its cross-section, a tunnel's portals,
  a bridge's piers by their height), one pass along the line
  could choose the cheapest run of each,
  with only how deep the railway can shape the ground as a limit.
- Rivers and roads along curves, in pieces and from `near`, as the path is,
  not as tubes:
  a tube makes all its points when made,
  and a shape cuts only its own thing, where a river digs the ground.
- Test only the sites of a row a shape can hold:
  a shape tests every site of its box,
  mostly empty for a slanting capsule,
  and a tube's against each piece of the level drawn
  (bent, the forest's branches cost a few percent more at level 0).
  A span per row from the pieces' boxes would keep the test exact.
- More trees as the oak (`Bole` and `Crown` in `worlds/src/forest/trees.rs`):
  the birch's trunk round, bent where its boxes step, its branches forking;
  the spruce's trunk round.
- Solve what a coarse cell decides (a block's lots, a room's furniture)
  with a small search whose boundaries the coarser band fixes:
  constraint solving stays local and needs no backtracking across cells.
  Lab's WFC sampler did so on squares (`lab/src/synth.rs` at 114c077),
  in rounds: corners from their four parents, then edges, then insides.
