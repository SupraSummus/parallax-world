# To do

Known problems, measured or derived, by the part they are in, then open ideas.
Numbers are for the forest at 1280×800, as measured when written.

## Generating

- **worldgen still decides some things with floats.**
  Shapes are kept and tested in fixed point,
  but they are given in floats, and their areas, fills and tubes' radii are f64,
  as is the forest's geometry (sin, cos, pow, exp, atan2, hypot, from `js.rs`).
  They should go to fixed point like the noise,
  the forest building each thing relative to its anchor.
  The fixed-point containment tests cost about 15% at level 0.
- **A box's faces round to the sites.**
  Faces on a plane of a level's sites split its sites ("Faces" in
  `docs/generators.md`); off those planes a face rounds to the sites,
  up to half a site either way: a town's floors at one height all go one way.
  A field's solid spans no longer do: their sites hold the share they cover
  ("Cells hold what they look like", below).
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
  by pairs of neighbouring columns, as rays meet voxels (`pairs` in `generator/forms.rs`);
  counted by columns, walls and rails hid a half to two thirds as much,
  which offset this: since, at level 4 against level 2,
  rays get through about 0.005 less often around houses
  and 0.01 less around the railway, on average.
  An opacity per line of nearest neighbours
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
  A site's pieces now may stop and glow unlike along each of the 12 directions
  (`VoxelGrid`'s `views`), so far only for a box's or an ellipsoid's sheet;
  straight down is none of them ("Cells hold what they look like", below).
  Still to do: rods, measured far forms.
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
  (`levels` in `WORLDS`: the forest's is 6, so 16 m),
  so `SURFACE` could be read from it, or `Column` could know it.
- **A thin ellipsoid between two planes of sites can vanish.**
  Its thickness grows to half a site either way of its centre,
  so centred between two planes it holds their sites only at a point:
  discs 2 units thick, each 10 units over the ground, were gone at level 2.
  Boxes keep the sites on their faces by `Tie`.
- **Some fields still keep their levels by rules of their own** (`plan::spacing`):
  the moor's stream bed and the railway write out
  `Column`'s rule of two sites (`.max(2 * spacing)`);
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
  - Boxes. Their three axes' slabs taken as if apart (tried)
    are wrong for what is thin along two, a rod or a speck:
    a small lamp's glow grew with the level.
    Wanted: the shadow of the box's part of the voxel along each direction.
  - Balls, tubes and gables; with them `Tie` and the thin shapes' rules go.
  - Spans of a medium or a coat, by how long a ray is in them;
    then `Column` need not thicken spans to two sites, nor coat thin media.
  - Hosts: the levels they ask for, the scale a body walks at.
    Seen from near, a level is pitted where its sites are held in part
    (a flat ground's top, a site deep),
    and sharp only where a shape is several sites thick.
  - Light and hosts see a site held in part unlike:
    light its share, hosts a whole voxel or none by the coin.
    Light corrects for the coin only next to what is shown (`shown` in `light.rs`),
    so two such sites in a row still shade what shows behind them.
    Hosts drawing pieces as light sees them would end it:
    blended by their alpha as a medium is (smooth, soft edges),
    or by a coin for each finer site in them (tried in `look`: both show no dark pits).
- **Layers that don't say where they are are drawn everywhere**
  (`Field::meets`, `Population::stands`):
  the city's buildings, and the towns', landmarks', settlements' and steadings' things.
  Saying so is for speed, not simplicity:
  a promise per layer says again what `heights` says once.
  The railway's piers can't yet: a bridge has no greatest height,
  and `About` is a band around one plan.
  Simpler would be a field of `Column`s on a plan knowing where it is,
  with only what varies by column (water, mist, bridges) declared.
- **Whole sites are still put one by one.**
  Under the surface nearly every put is a whole site of the ground's fill,
  each through `put`, its brick looked up, at level 0 its shade hashed.
  A line's run of them put brick by brick would spare most of that.
- **A site's glow and views are kept in maps by site** (`VoxelGrid`),
  so `generate` marks apart which sites may be in them (`odd`),
  not to look each up as it adds to it.
  Kept by brick, as the sites are, the grid would know itself.
  Sites unlike through their faces are the dearest to put:
  few of a tile's, each added face by face
  and taken out of the map and put back,
  they take several percent of the valley's time at level 0.
- **Plans are made at every column read, whatever their scale.**
  The valley's land, decided at 4 m, makes its six domains' bids
  at each column of a tile at level 0.
  Bids made every 4 m and interpolated (tried) cost as many instructions:
  most of a bid is the terrain, which the ground makes there anyway;
  level 0 took a tenth less, level 6 a quarter more,
  as things read the land at a point, four samples each.
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
- **Sparse populations anchor every cell first.**
  The moor's boulders (one cell in ten at most), the forest's boulders and logs
  make the ground at a point of each cell, and ask its land,
  before the cell's roll says no: the moor's took a sixth of the valley's level 6.
  Rolled in its anchor, that level cost a twentieth less,
  but split what decides a boulder in two, each with its own stream.
  A population's highest chance, rolled by the engine before anchoring,
  would serve them all alike.
- **Hashing is the forest's largest single cost.**
  Each site hashes three times (shade, pick, `shown`'s coin),
  each `hash64` mixing three times, as do noise corners.
  A cheaper hash would change every world.

## Checks and cost

- **Cost is counted for more than hosts make.**
  `cost::report` and the baselines count `generate` of what `shown` leaves,
  not `generate_shown`, which skips what is buried under each block (`Floors`):
  they don't see what that saves, nor what finding the floors costs.
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
- **Far regions and cost's places are still listed by hand**
  (`Entry` in `worlds/src/lib.rs`).
  The survey could give both: wide boxes around fields' strata,
  the places of the costliest strata.
  Then `Region` would go.
- **Things' variants of one part, drawing the same kinds, are one stratum**
  (`worldgen::checks::survey`): a ruin and a watchtower, both of stone,
  are checked in one region between them.
  Naming a landmark's node by its variant, as towns name their children,
  would part them; or the survey could count which code drew each part.
- **The solids' scenes ask little against the level two finer.**
  In `worldgen/tests/shapes.rs` every solid keeps the contract there
  to the last level checked: a `Look` is a mean over a region's rays,
  so a solid that thins changes it only by its share of the view.
  Scenes the solid fills, the same share for every case, would ask more.

## Worlds

- **A domain can't stand in a world twice.**
  Its populations seed their cells by layer name and the generator's seed
  (`Cell::random`), not its `Scope`: two forests would grow the same trees.
  And its kinds' ids are its place in a table every world concatenates
  in one order (`kinds!`'s `base`); named, the world would number them.
- **A domain owns a column's things, cover and air at once.**
  So a landmark's clearing, kept for itself to keep trees off,
  cut the moor's heath and mist away.
  The moor now lays its heath over the landmarks and its mist over them
  and the railway, by its bid without them (`moor::Alone`),
  but a landmark among fields or dense forest paints its own grass.
  Land could share cover and air apart from things,
  a domain bidding for its things only where it builds.
- **Worlds share tables of kinds**, so the contract only prints
  the kinds nothing surveyed draws, and fails on silent layers alone:
  the moor's, the learned town's and the city's are the valley's.
  With only the kinds it draws, a world could fail on a kind
  drawn only far from spawn, or never.
- **Worlds are not yet crates of their own.**
  One should be written without touching another world or the engine,
  using others' pieces as crates.
  Only the cheese is one (`worlds/cheese/`),
  and it still needs its entry in `WORLDS`, its name in three tests' `worlds!`
  and its cost baseline in `worlds/` (generators.md, "Checks").
  With `Entry` and `Region` in `worldgen`,
  each world's crate could give its entry and test itself
  (one macro of `worldgen::checks`), `WORLDS` a line per crate.
  Sharing pieces runs into the two items above.
- **The railway's track is too bright seen low from level 2.**
  At level 0 its sleepers and rails hide the ballast from a ray coming in low;
  at level 1 a medium of them over it does (`railway.rs`, as `Column::medium`);
  from level 2, shorter than a spacing, they are its top's dithered shares,
  as seen from above, and the grey ballast shows from every side.
  A field giving its sites a kind per face, as a thing's far form does
  (`Look`'s faces, measured in `generator/forms.rs`),
  would keep the ballast's shares on the faces looking up
  and the sleepers and rails on the others.
- **Generating the forest's grids takes 3–8 s of worker time per level**
  (the web's WebAssembly, in Node; the TS forest took twice that),
  most at levels 0 to 4 (a moved grid, a seventh of that).
  In the TS forest it was mostly the ground's noise,
  which the terrain and the canopy compute about 14 times per column
  (with its neighbours, for slopes),
  and at levels 0 and 1 the undergrowth's many small shapes;
  worldgen's, counted ("Cost" in `docs/generators.md`),
  goes mostly to the trees at levels 0 to 2
  and to the canopy from level 4 (`Rooms::owner` making the trees near).
- **One of the valley's tiles is over the cost budget**
  (`OVER_BUDGET` in `worlds/tests/cost.rs`), by things done one by one
  where a tile holds four times as many each level:
  at level 3 the towns' things drawn whole to be measured.
- **Two of the cheese's tiles are over the cost budget**
  (`OVER_BUDGET`): at level 7 a tile holds a few hundred thousand holes,
  each a shape, most of them a site or two
  ("Far forms for what fills space", below);
  at level 3, the grottoes each make the holes around them to place their stairs.
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
- **Towns and the city line up with the lattice.**
  `worlds/tests/directions.rs` lists them (`KNOWN`).
  The valley's first town lays out streets and blocks along x and z,
  and builds in frames of quarter turns (`frame.rs`):
  a four-fold harmonic of 10 %.
  The city is boxes on a plan, all along the axes: 4 %.
  The learned town's settlements are such boxes too (2.5 %),
  but their streets' own direction (2 %) hides them from the check.
  Square noise (`ValueNoise`, `Fbm`) draws under 1 %, too little to tell.
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
- **README's valley shots predate the winding valley** (`docs/shots/`).
  The view down it and the ranges were taken at fixed cameras
  where the valley ran along x = 0; retake them (`pnpm shot`).

## Light

- **Light is a prototype, in look's pictures and native's computed lighting**
  (`worldgen/src/light.rs`); the web does not light its tiles.
  [docs/light.md](light.md) says what it gets wrong against its reference,
  what was tried and what it costs.
  Where it is to go, a step at a time, each measured on the reference's scenes:
  sites that hold what they stop along each direction, boxes' too
  ("Cells hold what they look like", above);
  then coarser levels taking more directions (in Ideas).

## Native

- **The native frontend is missing pieces.**
  Its camera is f64, not fixed point (positions reach the GPU relative to it).
  Its compositor needs indirect draws counted on the GPU
  (`MULTI_DRAW_INDIRECT_COUNT`), and so Vulkan:
  Metal and GL lack them, and on DX12 wgpu's leave out the first vertex
  the chunk's slot comes as (`BACKENDS`).
  Its frames are bound by the GPU (`--stroll`):
  culling the clusters in view costs about a third of a frame,
  drawing those that show most of the rest.
  The first phase reads every cluster only to find those that showed;
  a list of them, kept from the last frame, would spare that.
  The pyramid of farthest depths is made over all its texture,
  each level a power of two: at 1280×800, twice the texels the screen needs.
  Walking in the window for minutes, a frame sometimes takes seconds
  until the driver resets the GPU (seen on Intel UHD with Mesa);
  the window then makes its device again (`window.rs`).
  Why is open: the same views and paths, drawn headless, take milliseconds.
- **Native's cells could be simpler and cheaper**
  (section 1 of `docs/architecture.md`).
  Nearly half the cells baked come back empty:
  they hold voxels (`Level::meets`) and are not buried by whole bricks
  (`Level::buried`), but hold none a ray of their face can enter.
  From the eye about one in seven of a frame's chunks shows
  (the valley's spawn, a colour per chunk);
  the rays that hide cells before they are baked (`Hiding`)
  leave about two in three of them at the spawns, in relief,
  four in five drawn flat, whose pictures reach past their cells.
  Of those they hide, about one in seven the GPU's probes find showing,
  most because a probe's box shows where its voxels would not;
  the first frame takes up to a third longer, waiting for probes.
  The rays are cast once a pick, from where the eye was:
  what the eye comes to see as it moves within it shows a few frames late.
  Cast again as it moves, they cost tens of milliseconds a face;
  cheaper rays (steps a voxel long, empty cells of 8·FCC crossed at once)
  could follow the eye.
  Mist hides nothing: the GPU's depths have none, so it would ask for it all.
  A server handing each player only the cells it may see
  would need, for each probe a client says shows,
  the pixel `sees` found, to check its ray through the voxels itself
  (as `Hiding` casts them).
  Cells straddling where a face's levels meet are drawn at both.
  Each render tests every cell a face composes against the screen,
  though a face keeps its cells picked (`Lod::pick`);
  the render that picks them anew, every few units moved,
  takes a few times longer.
  `surface` still finds a cell's cut faces, which `cell_relief` drops.
  Detail sets Q = max(4, ⌈2·detail⌉), so below 2 it changes nothing;
  Q itself, a whole number, would say what it does.
  The web still draws slices, with runs and cut faces:
  its compositor would need depth by distance too to draw cells.
- **Native pillars more than double the splats.**
  Natively a run is a row of splats, one a voxel
  (section 4 of `docs/architecture.md`),
  which in the forest adds more than half to the GPU's frame.
  Splats of a row that no ray of the face can see,
  hidden by neighbouring columns with runs as long,
  need not be made.
  Fewer, larger splats cost more, not less:
  a splat every other voxel, grown to leave no gaps
  and with its depth varying along the axis, culled worse;
  one splat cut to the hull of a pillar's ends drew large rectangles,
  mostly discarded.
- **Native's threads each keep their own plans' columns**
  (a generator per thread, `native/src/world.rs`):
  completing the valley's first view takes four threads
  about a tenth more work than one, eight a quarter more.
  Tiles side by side still make again the things reaching both;
  those one above another are made together (`make_tiles`).
- **Walking, native keeps more than a core busy**
  (the valley, `--stroll 20 --on-ground`, outside the GPU driver),
  measured when it drew slices: new tiles took about a third.
- **Native's voxels, lit, are lit slower than the eye moves**
  (the valley, `--light 1,1,0 --stroll 8 --speed 100 --by-work`, and walking):
  four threads, the cores to themselves,
  owe the view hundreds of bricks all the while,
  so each level shows lit less far than it reads, the next coarser past there.
  Nearly all of their work is then the light, a quarter of it its blocks;
  no part of it stands out ([light.md](light.md), "In native, and what it costs").
  Its tiles come in time, made before the bricks are lit.
- **Native's medium costs a fragment per slab a ray crosses**
  (architecture.md, "See-through"), as far as the view goes:
  pictures laid far to near, nothing behind what all but stops a ray is left out.
  Only media are (`KindDef::medium`): the moor's mist, the fog's (`world=fog`),
  where a pixel takes some 10 slabs on average and up to 40,
  and the valley's sea (`mountains.rs`).
  - The sea's top is a medium of its own, so that where it ends shows;
    it mirrors no sky.
    A kind's shine, kept by sites as glow is,
    would let a ray take the sky's colour where shine changes along it.
    Not looked at in the chunks engines, whose slabs blur where a medium ends.
    Ponds, the moor's water and wells are still whole voxels.
  - Most of what a fragment costs is reading its four columns from storage:
    two columns an item, or the pictures in a texture the GPU filters,
    would read half as often.
  - Its deep slabs (`Lod::for_medium`) show mist on the ground above and below it
    in faces with tilted axes: a V at their seams near the horizon,
    and a column of smoke, as wide as a slab is deep, cut across at a seam.
    Taking each half slab at its own middle plane halved the V at no cost,
    but needs four columns of neighbours round a picture, not one.
  - The medium starts at the first half slab past the near plane:
    right by the eye, mist is a little thin.
  - A site's pieces, where shown, hide its medium and what is behind:
    right on average, but a coarse site of a crown in mist
    is a whole voxel or mist, never the crown's grain seen through mist.
- **Native's voxels show what rays ask for a frame or two late**
  (architecture.md, "Native: another engine"):
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
- **Native, lit, holds three quarters more than it does shaded**,
  and more as the eye moves:
  the light's blocks (`Shared`) are kept wherever tiles are,
  though a brick lit needs them no more, till one beside it is lit;
  and each of the world's threads keeps sites of its own
  ([light.md](light.md), "In native, and what it costs").
  Keeping only the blocks read last (as `Recent` keeps rounds) was tried:
  held to fewer than a first view reads, lighting slows, by blocks made again.
  So was one `Recent` for all a Light keeps of a site,
  its paths and settled light with its rounds:
  as fast only once it held as much as now.
  Kept by all the threads, as the coarsest level's sites are,
  each would be found once, not by every thread that lights beside it.
- **The light's blocks of level 3 cost several times their neighbours'**,
  a block, lighting the valley's first view:
  those a town reaches,
  where each of its buildings is measured for its far form
  (`measure` in `forms.rs`), once a box, whatever the box's size.
  Few are measured twice, so keeping them would not help;
  measuring goes over all of a thing's box, most of it empty.
  So `voxels_show_the_faces_rays_enter_by`, lit, takes over ten minutes
  on a software GPU's machine: its one block, 512 wide, is measured for each box of light.
- **Native's voxels wait on memory** (`march` in `voxels.wgsl`):
  most of a frame is its samples' rays, some 50 steps each,
  and a step costs its reads one after another (a brick's slot, then its site),
  not its arithmetic.
  On Intel's GPU (Mesa) the shader runs 8 pixels a thread, not 16: too many registers.
  Tried, and no better: stepping by increments;
  cells of 128·FCC (a window's torus is 4 of them a side,
  so cells a window apart share a count: the sky the ground's);
  each empty voxel saying how clear around it is
  (a third fewer voxel steps, most turned into jumps: 2%, tiles twice as slow to make);
  a rhombic dodecahedron about what a window holds, for its box or with it
  (its faces slant across ground that lies flat in the lattice's axes).
- **Native's voxels while the eye moves** (`Windows::sync`).
  - A frame steps one window, each in turn,
    so level 0's ball moves a brick every ninth frame:
    flying faster, it lags, read less far, till it starts anew.
    Stepping first the window that holds least of what it should around the eye
    (`Ball::about` against its radius), not each in turn, may do better.
  - Walking, frames come every 33 ms:
    CPU and GPU together take a little more than the 16.7 ms
    the compositor paces frames by.
  - Making the uploaded bricks' words (`words`) varies tenfold from frame to frame,
    by memory, not work.
  - Shots and tests wait for the windows a tile at a time,
    a whole picture drawn each, so the voxels' GPU tests take twice as long.
- **`culling_on_the_gpu_leaves_the_picture_unchanged` failed on an Intel UHD** (Mesa),
  drawn flat, every time: the forest from its spawn.
  Likely by ties in depth drawn in no set order, fixed since:
  lavapipe fails there too with the draws reversed. Untried on Intel.
- **Native's flat pictures** (`--engine flat`, architecture.md, section 1).
  - Their depth is by w, ahead along the view (`flat_depth`),
    the other engines' by distance from the eye (`depth_of`),
    so culling and the pictures of medium ask which the frame draws.
  - Their runs are cut to their face's rays by clip planes,
    which cost Intel's GPU about as much as all else they do
    (the valley's frame 5.2 ms, 3.6 without).
    Uncut, faces' pictures overlap past their seams, and tie there (below, "A shot may differ").
  - Neighbours in a layer of cells often lie on one plane
    (cells of one column always: `STEEP`),
    and enlarged (`Flat`), one shows over much of the other's picture
    where the ground runs along the axis, its pillars through the cell:
    they tie, and the one drawn last shows.
- **Native's chunk engines while the eye moves.**
  - Their renderer composes a frame behind (`chunks.rs`),
    so `--stroll` counts nearly every frame as one whose content is still coming:
    it no longer tells holes from that;
    `--replay --shots` does, from a walk the window recorded (`--record`).
  - Turning leaves holes: a face out of view has no pick,
    and one coming into view is baked from nothing,
    tens of thousands of chunks, seconds of the bakers' work.
    Picking faces out of view too would add to a queue
    that walking already keeps from emptying.
    Drawing coarser cells where finer ones are missing (from the near plane on)
    filled the holes, but drawn flat, a coarse picture seen that near
    stands into the sky.
  - A pick moved composes its every chunk again, on the renderer's thread,
    and thousands of chunks come and go at once,
    most of them the finest cells', a voxel or two each:
    the compositor spreads them over frames (`CHANGES`),
    a few microseconds each on the main thread.
    Fewer, larger chunks near the eye
    (a chunk holding a cell's finer cells' pictures too) would cost less.
  - A shot may differ from run to run (flat, the valley):
    where depths tie exactly, the one drawn last shows,
    in the clusters' order: the order chunks came in, which timing decides.
- **Native's splats may go too.**
  Each whole voxel a hexagon is the voxels engine's idea, rasterized.
  They are worth keeping only if faster than reliefs on some GPU,
  as on Intel's, which tests depth only after a relief's pixel writes its own.
  Gone, `Chunks::new` takes `flat` alone, and the hole below goes with them.
- **Native's splats leave a hole by a hut.**
  `--engine splats --view 'seed=42&world=valley&camera=1200,100,36700&look=20,-12'`:
  sky shows through the ground in the bottom right, with or without culling,
  in straight edges like a cell's. Not looked into.
- **Native's reliefs.**
  - Thin things seen at a slant break at the default detail:
    the city's road markings into combs, whole at twice the detail.
  - Marched per pixel, a tile per quad, they are slower on lavapipe
    than whole splats (`--engine splats`), which lavapipe tests no earlier;
    a real GPU is unmeasured.
  - Tiles hold their empty columns: about half of those uploaded.
  - A few pixels show what is under the ground at cells' edges
    (7 in the forest at `seed=12345&camera=-14,40,-200&look=-25,6`):
    columns' cells are not their voxels' outlines.
  - Where pillars of two cells meet a voxel at one site,
    the one whose front is nearer it shows (`shown` in `compositor.wgsl`);
    as near, they tie, and the one drawn last shows, its front's colour.
  - `chunk_reads` reads a cell deeper, for the solid behind a relief.
- **The native compositor's buffers stop at the device's limit.**
  `Arena::grow` caps a buffer at what the device allows
  (128 MB on lavapipe), and `allocate` then hands out room past it:
  wgpu panics in `Compositor::prepare`
  (the city at 1280×660 at detail 27 did, before `GRID_REACH`;
  the forest at its defaults does, `seed=42&world=forest&look=200,10`).
  Past the limit it could drop the farthest chunks, or split the buffer.

## Web

- **Slice detail sets both thickness and level of detail.**
  "Slice Detail" sets how thick slices are,
  and so their level of detail (atoms of 2^(k−s) at level k,
  s as large as the grids' reach allows).
  Thicker is faster but coarser: in the forest, a first render takes
  about 6.5 s at detail 9.5, 3 s at 2.4 and 2.7 s at 1
  (voxels 4 and 10 times as large), against 2.5 s drawn flat at 9.5
  (Node, 1280×800).
  Thickness could follow what culling and the slice count want,
  and the level of detail the screen's pixels:
  coarser atoms than detail gives are a smaller s,
  finer ones need more than 2^4 layers of atoms a slab,
  so splats' depths in more than a byte (`ATOM_SHIFT`).
- **Coverage is weaker in relief.**
  A splat moves on its centre plane with the camera,
  so a chunk's mask holds what it covers from the axis and the corners
  of the rhombus of rays, and what it may reach from any:
  in the forest at the default detail, twice the chunks of flat slices are drawn.
  Nor is that exact: a block covered from each of those may still open
  between them, which no picture has shown yet.
  The camera's slant is known where coverage is tested;
  masks for a few slants, picked per block, would cover nearly as much
  as flat ones, and between nearer slants.
- **Coverage masks are coarser than the screen.**
  `mask_scale` marks ⌊40 / t⌋ pixels per unit of a slice t thick,
  but at a level's nearest slices a unit is focal / (t · detail · LAYER)
  pixels on screen, about 95 / t at 90° on 1280 px at the default detail.
  A mask pixel counts as drawn when its centre is in a splat,
  so a gap between splats narrower than a mask pixel can count as covered,
  and what is behind it is skipped.
  The mask could follow the focal length and detail,
  keyed in the chunk like its grid, leaving the slices as they are.
- **The web culls only whole clusters, on the main thread.**
  The forest has millions of splats in view, and only about 1.5 % show a pixel;
  coverage leaves out the clusters hidden whole,
  and the rest are drawn.
  The native compositor culls finer, on the GPU
  (section 4 of `docs/architecture.md`), which WebGL 2 cannot
  (no compute, no indirect draws);
  WebGPU can, where a browser has it.
- **A chunk as an image was slower, on an Intel GPU.**
  The branch `relief-chunks` draws a chunk's columns as tiles of
  colour, depth and run, a rectangle each whose fragment shader
  follows each ray through them, or traced in compute:
  right to the pixel, runs included, a tenth of the vertices,
  but slower than splats in the forest.
  There (Mesa's driver), a fragment shader that discards
  or writes its depth, even conservatively, is depth tested after it runs,
  so it runs for several times the fragments that lie before what shows.
  GPUs that test early despite discarding may differ;
  fewer, thicker slices, a column holding several runs,
  would cut the layers a ray crosses.
- **The depth within a layer needs a fine depth buffer.**
  A layer's splats share 1 / (layers + 1) of the depth range;
  WebGL promises only 16 bits, which leaves a few dozen steps for them.
- **Bricks on the edge of a face's rhombus are baked whole.**
  Near where a level starts, about 19 bricks meet the rhombus,
  and most of them cross its edge or the screen's.
- **Most baked splats are hidden.**
  When chunks were bitmaps,
  over 95 % of opaque baked pixels were covered by nearer slices.
  Content that no ray of the face reaches within its slice
  need not be baked.
- **Flying, the main thread can't keep up with the workers.**
  Flying up (a trace in a desktop browser),
  workers bake about 4,000 chunks a second
  and the main thread is busy 97 % of the time, at 10–13 frames a second.
  Renders take 30–90 ms, longer the more new chunks they take:
  uploading textures (now splats), handing chunks to `Coverage` (`coverage_add`),
  and covering with them (`coverage_cover`).
- **Turning, finding covered chunks is slow.**
  About 11 ms of a render (forest, 1920×1080, Node),
  mostly testing sub-blocks one by one in `cover` (`web/wasm/src/coverage.rs`):
  about 270,000 tests a frame, three in four failing,
  as the forest covers the screen in fine pieces.
  Testing quarters of a cell first would not help (two in three are mixed),
  nor would clipping without allocating in `shape`.
  The web could run it in a worker, a frame behind.
- **After a long jump, the workers' frame culls more than the main thread's.**
  Once the grids of three or more levels move at once,
  a renderer baking in workers counts a few more chunks covered
  than one baking on the main thread at the same view
  (`worker-bakery.test.ts` keeps its move short of that).
  Whether it shows on screen is not checked.
- **Moved grids are posted whole.**
  When a grid moves with the camera (section 1 of `docs/architecture.md`),
  the main thread posts all of it to every worker, though they have most of it:
  at level 0, ~40 MB a worker every 12 m walked.
  Workers could keep the tiles and put the grid together themselves.
- **Far voxels 5.6–11 pixels across.**
  Atoms 4 times as fine (1.4–2.7 pixels)
  would need `GRID_REACH` 4 times as large:
  ~16 times the grids' bytes, in each bake worker.
  Grids shared between the workers would allow it,
  or bricks sent to a worker only once a chunk it bakes needs them.
  (Shared memory needs cross-origin isolation,
  which GitHub Pages can only get through a service worker.)

## Tools and CI

- **TS and Rust still say much the same twice.**
  The tiles around the camera (`world.ts`) are `native/src/world.rs`,
  and faces are in `native/src` too, though its renderer draws cells, not slices.
  The renderer is the large step.
  Tests too: cells' and view's are in both;
  relief's, pillars', the renderer's and its regressions' only in TS.
- **Tests take most of CI's time.**
  Most of it is the worlds' contract, bound by the CPU, in jobs of its own;
  the valley's, alone, is CI's longest job.
  Its threads end together, so only less work would shorten it,
  or its seeds on several runners (`--nocapture` shows each part's time).
  A seed's margins (rays, and pictures from above) take longest,
  then checking its regions in parts.
  Pictures from above generate level 0 over all of a world's heights,
  region by region, keeping the tiles a region shares with the one before:
  the forest's regions overlap, the valley's hardly.
  If CI is still too slow, the contract at seeds but `REGIONS_SEED`
  (`worlds/tests/contract.rs`) could run only on main.
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
  - `worldgen` holds more than generating, none of it read by the engine:
    `checks/` (only tests and examples use it), `light.rs` (only native and `look`),
    and in `picture.rs` a PNG coder, of which `lab/src/png.rs` is another.
    Crates of their own would say so; `picture.rs` in parts.
  - `worlds/src/forest.rs` is still a world, the ground others build on
    (`Relief`, `Forest`) and the table of kinds they draw with (`K`);
    the valley's domains (`worlds/src/valley/`) use each other in circles.
    The kinds and the ground apart
    come before "Worlds are not yet crates of their own".
  - `worlds/data/cost/valley.txt` is the repo's largest text file
    and changes with most commits to the worlds;
    a baseline per place and level would do for a test with a quarter's slack.
  - `docs/generators.md` holds the worlds beside the engine ("Composing"),
    and `docs/architecture.md` native's cells and voxels inside the web's sections.
  - Tests at the end of `picture.rs`, `light.rs`, `native/src/main.rs`,
    `voxels.rs` and `lod.rs` are a quarter to a half of each;
    `generator/tests.rs` shows the way.

## Ideas

- Components' baselines are snapshots outside git (`worlds/examples/check.rs`);
  kept in git, a component's would be checked as its world is,
  and its accuracy could be declared next to its code, not in `WORLDS`.
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
  Light keeps 12 at every level, too few far off (a lamp's lobes, blurred shadows).
- Slices' splats that show alpha (the web's show no medium,
  so the moor has no mist there):
  a chunk keeps a column's voxels till their alpha adds up to whole,
  and splats that overlap in a slice may add up to an opaque one,
  the rest drawn see-through.
  Alpha in steps of optical depth (−ln of what gets through),
  which add up where things are apart, might take fewer bits.
- WGSL formatted in `pnpm lint`, as rustfmt does the Rust.
  wgslfmt 0.1 (wgsl-analyzer's) pads compound assignments (`x   += 1`)
  and joins long expressions into one line; try a later one.
- Report cost at the places worlds name (`World::points`),
  not at the spawn and the regions' centres (`cost::places`),
  once the forest, the moor and the city name some:
  they would be measured at their spawn alone.
- Assume no ground.
  y stays up: bodies fall along it, the sky and the sun are over it.
  But the engine still takes a ground where a world has no say in it:
  - `shown` trims a box along y alone, and `Floors` find what is buried
    from the bottom up (`on_rock` keeps it).
    Halved along every axis it would not pay:
    asked brick by brick, `whole` cost many times generating the tile.
    Blocks of columns finer than `FLOOR` would take a ground's range
    from columns between the level's own, each made for it.
  - The web tiles x and z over a world's heights (native's tiles are cubes),
    so it can't show the cheese.
  - Places and the survey sample lines of cells along y (`PER_COLUMN`);
    `directions` measures the four-fold harmonic about y alone.
- Far forms for what fills space: the cheese's holes are carved
  while wider than the sites are apart (past level 8 it is all buried),
  and its grottoes (pillars, stairs, crystals) stop at level 3,
  and its only light with them:
  a far form glowing on the caverns' walls would keep it.
  Holes as things that cut the world cost far more: each a thing, its cells asked.
- Lay out towns at any angle?
  Walls along the lattice's axes look crisp on FCC;
  at other angles they jag.
  But every town and the city runs north–south and east–west,
  which at the scale of a layout reads as a grid.
  A middle way: streets and plots at any angle, the buildings on the axes.
  The buildings' walls would still show to `directions.rs`.
- Keep the fit of `cost::NS` in the repo, to refit it on a quiet machine.
- Generate the web's tiles as the native frontend does:
  those the chunks in view read, nearest first,
  instead of every tile within `GRID_REACH` of the camera.
- Keep the view in the address bar as the camera moves,
  so a view can be copied (the page reads it already).
- Measure the seams between faces against ray-cast perspective.
- Make the 12 stacks cheaper:
  four or more faces are in view at once, each with its own near slices.
- Find covered chunks more exactly: many chunks drawn show no pixel.
- Measure the WebGL compositor on real GPUs (headless Chromium only has a
  software one); if draw calls matter, pack chunks' splats into one buffer.
- Drop a splat's size: every splat is a voxel's outline,
  so a splat could be its site, depth and colour.
- Show render distance in metres:
  they are lattice units, and the forest's unit is 0.125 m.
- Show the forest to 22 km again: `MAX_LEVEL` 13, one grid more.
- Learn a world from examples instead of the split grammar.
  Two models in lab draw any box alone, flat or on the FCC lattice.
  The masked model (`lab/src/masked.rs`, `masked_fcc.rs`)
  learns the statistics and new layouts,
  but breaks long thin things (lanes, tunnels) into pieces.
  Synthesis by coordinates (`lab/src/exemplar.rs`, `exemplar_fcc.rs`)
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
- The critic judges flat maps only;
  3D scenes are still scored by pooled statistics (`lab/src/fcc_stats.rs`).
  Its windows as volumes would judge the FCC models too.
- Join lab's cubic and FCC samplers with one lattice type
  (sites, neighbours, parents): each pair now repeats its box bookkeeping.

Composing generators (section "Composing" of `docs/generators.md`), untried:

- More populations at `Points`, their far forms from what stands:
  the forest's boulders, logs and plants, the valley's haystacks.
  Moved alone they cost as much:
  their chances, read at every leaf's corners, read the land
  as their refused anchors read the ground.
- Bid per quantity: a land per channel (use, ground, wetness),
  each domain bidding in those it cares about, as the towns' terrace needs.
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
- The mountains' gorges (`Landforms` in `worlds/src/valley/mountains.rs`) are dry,
  with no stream in their beds,
  and below the treeline the forest covers their walls and the spurs' noses,
  where rock would show.
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
  On the FCC lattice, a fine site in six coarse cells, two or one
  would be a corner, an edge or an inside.
