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
- **`Column` grows its layers to two sites**, a patch
  ("Cells hold what they look like" in `docs/generators.md`).
  Its reason, as `column.rs` and "Spans that stay" still give it, is gone:
  a span under two sites could hold no site,
  but every site now holds what its spans stop of its voxel's chords,
  however thin (`cover` in `drawables.rs`).
  A layer of turf a unit thick is 16 units at level 3,
  so a slope shows turf from the side where its soil is.
  Taking the rule out changes every ground: not tried.
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
  - Spans of a medium are held so now (`held` in `drawables.rs`). Left:
    - A see-through kind has one share a site, pieces one a face,
      and a site shows its pieces whole or not at all.
      So it takes its most across the line,
      and a painted one takes the pieces' place through every face alike (`see`).
      Its own share for each face, drawn so by hosts, would end both,
      and the thickening below.
    - One standing free (`Span::medium`: mist, rails) is still spread over two sites:
      in the site of the ground under it, the ground's pieces, shown, hide it.
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
- **Sites unlike through their faces are the dearest to put.**
  A seventh of the valley's sites are in `views` (`VoxelGrid`),
  which take as much as all the sites' pieces,
  held by sight (`Sights`); each its own twelve, twice that.
  Half are alike another site's: held once, they took a third less,
  for a grid not to be changed once held. Not kept.
  Each is added to face by face (`pieces` in `generate.rs`),
  a dozen sums a put:
  lighting a small view, where most of the work is the coarser levels' tiles,
  that is over a tenth of it.
  Their glows are kept apart (`glows`), as few glow:
  one record a site would be read and written once, not twice,
  for memory not measured.
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
  And for boxes four of native's tiles a side, each alone (`cost::tile`),
  where a host makes a tile with those above and below it.
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
  Unlisted, a field is checked by its cells alone, which see little of a medium on its ground:
  each held to what is in it, to the world's `levels`,
  in a region its stratum shares with others (the sea's, with its beach).
  So the valley's far sites showed the sea's floor dry till its coast was listed.
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
- **Generating the forest's grids took 3–8 s of a thread's time per level**
  (in WebAssembly, in Node, when the web drew grids; the TS forest took twice that),
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
  Drawing one is a quarter of measuring it (`measure` in `forms.rs`):
  the rest counts its fine sites by cell, axis and face,
  with a vector of kinds for each face of each cell.
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

## Time

The prototype of worlds that move (`worldgen/src/time.rs`;
"Time" in `docs/architecture.md`): the valley's clouds.

- **Tiles are made anew whole, and that is most of what moving costs.**
  A tile the clouds are in is made anew each tick they cross one of its sites,
  all of it, a few ms each; the clouds' layer alone is over a third of that.
  Under the clouds, on four cores, over half the tiles with clouds that the voxels engine reads
  are behind the world's time (`--stroll --by-work` says).
  To try: making anew only the bricks that changed,
  which edits and physics would want too;
  the valley keeping its clouds' cover by where the wind has carried it (`recent.rs`).
- **The world, not the engine, decides what to make anew.**
  It makes anew each tile of another time within an engine's reach,
  though only the engine knows which show.
  To try: a tile of another time counted as lacking, and the engines asking for it.
- **Behind, tiles show different times.**
  Clouds tear along tiles' faces, and between levels.
- **Light does not move.**
  The light's blocks and what Lights keep (`worldgen/src/light.rs`)
  are made once, at time 0, and a tile made anew lights from them
  what the old one had lit and changed:
  a cloud's shadow stays where it fell.
  Each needs the time it stays for, as tiles have.
- **Only the voxels engine follows.**
  The chunks engines bake a cell once its tiles are in, and keep it:
  there the clouds stand still.
  A chunk stays no longer than the tiles it reads (`chunk_reads`).
  The web runs the same frames, with no threads: not looked at.
  Nor has the voxels engine placing tiles made anew been looked at in a window,
  only by its tests.
- **Tiles made together stay together.**
  Tiles one above another, made as one box, stay no longer than the clouds among them;
  made anew alone, the others then stay always: each is made twice.
- **Only drifting, and only by layers.**
  `time::drift` is all that reads the time, and plans and things may not,
  kept as they are from box to box:
  a tree's crown in the wind is a thing.
- **Time on its own lattice.**
  Time is a parameter of the world, not a dimension of its lattice.
  D4 (sites of space and time adding up to even, as FCC's do in space) would make it one:
  only for what moves, as what stays would be made anew each step.
- **The window never rests under clouds**:
  it draws a frame every 60th of a second while any tile holds what moves.
- **README's pictures of the valley are from before its clouds.**

## Light

- **Light is a prototype, in look's pictures and native's computed lighting**
  (`worldgen/src/light.rs`).
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
  as much as four gained: so `Kept` in `plan.rs` reads without one, unsafely.
  Tiles side by side still make again the things reaching both;
  those one above another are made together (`made_with`).
- **A `Large` never forgets** (`plan.rs`): it holds each piece of a curve for good,
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
  - The chunks engines take a medium's alpha for every channel,
    their pictures holding one optical depth a column;
    the voxels engine, the light and picture.rs's rays take each channel's own
    (architecture.md, "Native: another engine").
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
  `fcc`'s `Tiles` and the windows take cubes.
- **A level's tiles are indexed anew whenever one comes or goes**
  (`Level::new`), and small, they come by the dozen:
  flying lit, an update takes the main thread several ms at most.
- **A world lit only later makes its tiles twice.**
  Shaded, a tile's sites are not kept (`Queue`'s `lit`):
  the light, turned on in the panel, makes them again.
  One store of tiles for hosts, lit or not, would make them once,
  for what the sites of shaded tiles take;
  it would not be the light's then (`Shared` in `light.rs`).
- **How far past an engine's reach tiles are kept is chosen**
  (`KEPT` in `native/src/world.rs`),
  not derived from how far the light reads (`top_steps` in `light.rs`, and each level the next).
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
  - Shots and tests wait for the windows a step at a time (`STEP` tiles),
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
    `--shots` does, replaying, from a walk the window recorded (`--record`).
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
  `--engine splats --seed 42 --camera 1200,100,36700 --look 20,-12`:
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

## Browsers

- **In a browser the world is made on the main thread**,
  a piece or more between frames (`World::work`), on one core:
  a first view takes several times what the window's threads take,
  and a piece that takes long holds its frame.
  Threads there are workers that share the module's memory:
  a nightly toolchain (std built with atomics),
  a page isolated by headers GitHub Pages does not send
  (a service worker can add them),
  and no lock waited for on the main thread.
- **A browser draws by the voxels engine alone.**
  The chunks' compositor needs indirect draws counted on the GPU
  and depth tested before a shader that writes it,
  which WebGPU lacks ("The native frontend is missing pieces", above).
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
  A seed's margins (rays, and pictures from above) take longest,
  then checking its regions in parts.
  At `REGIONS_SEED` three threads each make the world (`worlds/tests/contract.rs`);
  one for all, now that a world is shared, would make its plans' columns once. Not measured.
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
  - `docs/generators.md` holds the worlds beside the engine ("Composing").
  - `native/` is no longer only native, and `bake/` serves one engine of it,
    the chunks', which a browser cannot draw.
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
  - Places and the survey sample lines of cells along y (`PER_COLUMN`);
    `directions` measures the four-fold harmonic about y alone.
  - Light takes its sky from above y (`SKY`, `SUN` in `light.rs`),
    and ends a line as far as the world's heights are high (`top_steps`).
    A world's generator could say where light comes into it from outside,
    at which sites of which levels:
    a sky over a ground would be such a world's, not the engine's.
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
- Measure the seams between faces against ray-cast perspective.
- Drop a splat's size: every splat is a voxel's outline,
  so a splat could be its site, depth and colour.
- Show render distance in metres:
  they are lattice units, and the forest's unit is 0.125 m.
- Show the forest to 22 km again: one level more (`GRID_LEVELS`).
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
