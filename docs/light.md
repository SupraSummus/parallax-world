# Light

Light on the lattices (the crate `light/`) is a prototype,
in `look`'s pictures and native's computed lighting.
The model is in its header (`light/src/lib.rs`),
and how native uses it in section 2 of [architecture.md](architecture.md).
Here: what it gets wrong, what was tried, and what it costs.
What is left to do is in [todo.md](todo.md).

## What it gets wrong, and why

Light comes along 18 directions only,
the 6 across a cube's faces and the 12 across its edges,
and only ever across faces: a step across an edge goes half through each
of the two sites beside it, then the one past it.
Off a whole voxel, where a site beside the edge is whole too and touches it,
all of it goes through the other: so open ground under the sky is lit
as it should be, not two thirds as bright.
Each level takes one step (`STEPS`; native's panel sets up to three)
before the coarser one takes over,
or more where the coarser fills what it leaves empty.
So light from D away comes from a level whose sites are about D across,
which keeps what a region stops on average (the contract),
not whether it is closed:
walls thinner than the space they close may let light in ("Thin walls"),
openings smaller than it let none (the door and the window below).

A railway tunnel, under 1 to 4 m of rock where its hill is low,
was lit through it from the sky, its middle hundreds of times too bright;
not measured since "Thin walls".
The reference's tunnel under a thin roof is half as bright as it should be in its middle.

Columns put a whole site where its middle was in a span,
so the coarse levels lost a roof thinner than their spacing
along some directions
(their solid spans cover their sites in part now; the tunnel was not lit anew);
and taking over, a coarser level's light comes from the face of its cube
the line leaves by, between it and the 3 beside it in its plane
(bilinear, weighed by how much each site, and the finer site towards it, lets through),
what lies between there and where it took over unread:
the same hand-off at every level of the octree.

Three steps a level leave the tunnel ten times too bright, not hundreds,
but the lamp several times further off.

The sun is one of the 18 directions.
Light taken over from a coarser level's single site was blocky,
a step at each coarse cube's face;
read between the four faces, it is smooth.

## Thin walls

Sheds closed by cuboids thinner than the coarse levels' sites,
some off the lattice, keep the sky out, as the reference does
(`thin_walls_keep_the_sky_out`).
They let in a tenth to a third of open ground's light, whatever the steps a level,
till two changes, which cost no time:

- a site is seen through the faces a line comes into it by (`meets`),
  past an edge through the one of two that stops more:
  the line crosses a wall in one of the sites it meets there.
  Seen through one face fixed by direction,
  a wall across z stopped a thirtieth of a line along (0, 1, 1);
  through both on average, the roof leaked as the walls had;
- taking over, a coarser site beside the line's is read
  only as far as the finer site towards it lets light through
  (`through` in `coarser`), so not past a wall:
  that let a tenth of the sky into the shed's middle.

Not looked at: walls slanted to the lattice, and their corners,
where lines cross sites that each stop part of a wall.
Tried before those changes,
each site's share along an axis made of its children's, lane by lane,
one to three levels down: no less light in the sheds.

## Where it is to go, and what was tried

Where light is to go, a step at a time, each measured on these scenes:
sites hold what they stop along each direction, a share, boxes' too
(light reads a site's share already, as hosts show it);
then coarser levels take more directions ("Ideas" in [todo.md](todo.md)).

Taking over later or less did not do, each tried against the reference:
only where the coarser level is open some sites ahead,
only the share of the light its sites around have room for,
or after two steps.
Each lit the closed room better,
and left the roof's middle and the room past the yard's door dark, or lit it many times too much:
there no direction of the 18 meets what is lit,
and the coarser levels' blur is all that carries light.
So what is left in rooms, under roofs and in the tunnel
is how few the directions are and how coarse a level takes over,
not how it does.

A site whose light is none in two rounds running is found dark (`change`),
though light may come to it after more bounces.

Rock filled round the tunnels kept them dark,
but bent a world to the light: not kept.

Columns and boxes generated anew for light,
their coarse sites covered in part along each direction,
left the tunnel a third as bright, still tens of times too bright:
not kept
([SupraSummus/parallax-world#264](https://github.com/SupraSummus/parallax-world/pull/264)),
as the world had two forms, the hosts' and the light's,
and each block was generated twice.

A beam along a direction enters a voxel by five faces,
a third of it by the one across and a sixth by each of four slanted to it
(on a cubic lattice, by the one);
light reads the site across the one, and `CROSSING` stands for the rest.

Tried, the five sites read, each a line of its own from there:
light went round walls one site thick, a third of it at each level.

Tried, the slanted four only stopping, glowing and reflecting,
what they let through the straight line's:
walls held, and without `CROSSING` the strewn blocks stopped what they should,
the lamp and the white room about as before
(closer with `CROSSING`, the blocks then a third too dark);
but the yard and the ground under the roof came out darker,
the yard several times:
the coarse levels' walls, a site thick, stop more of a beam five faces wide.
Not kept.

## Against the reference

Against the reference (`light/src/reference.rs`, test `against_the_reference`;
the ignored test `maps` draws the scenes from above), on cubes:
open ground under the sky, and under blocks strewn over it, as it should be;
a lamp's light within a sixth; under a roof, turned or not, within a third;
a wall's shadow in the sun too long, past where the sun is back,
as the levels that carry it get coarser;
past a door from a yard, where only bounced light comes,
a room four to six times too dark, and the yard a fifth too dark.
On the FCC lattice, the roof along it and the lamp were off by twice as much,
the rooms bounced light about as now.

In a closed white room or a cave, where light bounces many times,
the light on the floor is three to six times too little,
little more than reaches it from the lamp.

A lamp lights only as far as the coarser levels hold it:
the cheese's crystals stopped at level 3, and its caverns were black
a few metres from them.
Past it they are glowing pieces (`far` in `worlds/cheese/`):
on its walls, measured by hand, light is a third over the reference;
drawn as a medium, a sixth under.
Light reads the two unlike; the contract does not tell them apart.

A site met reflects the light on the face it is seen through,
a third of what it shows that way (`meets`; "Thin walls", below).

Tried: the light on all the faces it shows, each by its share:
no closer to the reference.
And with it a whole voxel touched read as any site met, not lit as the face:
a wall one site thick lit its dark side, and a white room was darker.

A whole voxel touching a face is taken to be lit as the face is
(`weighed` in `light/`), as a flat floor's are;
where it is not, at a wall's edge or a corner, the face is lit wrong.

It sends back no glow: a lamp's voxels do not light each other's faces,
so a lamp of white voxels is a third dimmer near than the reference's.

A coarse site holding part of a door (as `Far::measured` draws one)
lets light through in every direction alike:
drawn so, the yard's door made the room tens of times too bright.

A window smaller than the coarser levels' sites lets no sky in at all
(they close the room), so a room lit by its windows is black.

## In native, and what it costs

The world's threads each light what rays ask for with a `Light` of their own:
they share the world, its tiles, and the light found at their sites (`Shared`),
so a tile is made once, for what draws it and what lights it
(`Shared::make_tiles`),
and a site's light is found once, whichever of them asks.
A tile is made with those above and below it (`made_with`):
alone, a cube costs about twice what it does in its column.
The world draws grids that keep of a site only what is drawn (`VoxelGrid::to_grid`),
beside the sites the light reads,
till hosts read sites as light does
("Cells hold what they look like" in [generators.md](generators.md)).

What is found at a site is a function of the world alone,
so the Lights keep it together, every level's alike,
and two finding the same at once do no harm.
Each part of a site is set once, its rounds under a lock of their own
(`Room` in `light/`):
under one lock a column of tiles, and found by a map,
the threads waited for each other, and the first view took a quarter more work.
Their room is made a cube of 4 sites a side at a time (`CUBE`):
light begins few sites of most bricks it reads,
and of a brick's 512 a forest's first view began under a quarter.
Each Light holds the tiles it read (`Level`), asking the others for each once,
and lets go of a level's when that level forgets.
They keep the light of the tiles read lately (`KEPT_SITES`, `Shared::begun`),
each level by the sites it began.
Counted over all levels at once,
a coarse level's tiles went unread while finer levels worked,
as finer sites read its light once,
so they were forgotten and lit again, several times over:
a view took three or four times the work.
Fewer kept a level hold less,
but under what a view reads at once, lighting slows several times over:
a third fewer, and the forest's first view took half as much work again.
Chosen so that a view holds about what it did before.
So each byte a site keeps costs time too.
Forgotten brick by brick, not tile by tile,
a flight held a tenth less and owed twice the bricks.
What a site keeps is, most first:
the sites met that reflect, twenty or so, 8 bytes each (`Reflects`);
what comes to it (`Paths`);
its rounds;
and its settled light, of half the sites.
A site that stops no light keeps only its settled light (`Room`):
nothing reflects on it, so none reads its rounds.
What a site's light is found from is one allocation,
one pointer from its cube (`Site`).
All of it is in 16 bits a channel (`Packed`): half of what floats took,
a hundredth of a view's pixels one step of colour off for the rounds,
another for the settled light.
Room for each round alone, not a vector's to spare, holds a fourteenth less
for little more work (on the FCC lattice, a tenth more).

Most of lighting's own time is reading:
the world's sites the lines meet,
and the rounds of the sites met that reflect, each a few cache lines away.
Tried, and no faster:
the sites lines met last kept by where (a quarter to a third met again);
a grid's alpha and colour of a site kept together;
the sites met that reflect kept by site, so that each is read once a round
(a face's neighbour is met along its axis and beside its 4 edges):
under half as many reads, as many cache misses;
a tile forgotten keeping its sites' settled light:
what is lit again is mostly what that light is found from.

Native's light sliders light the world anew (`World::relight`),
as slowly as at the start.

Lighting the valley's first view, most of the world's threads' work
is finding the light, a sixth or so generating
(the tiles rays ask for and those the light reads around them).
Of finding the light, most is the rounds past a site's first:
each round reads sites one further around what is lit,
many where sites filled in part reflect on each other, as all through a wood's crowns.
Each site met that reflects has its own light found, and its tiles made,
and where many stop little (mist, crowns, the coarsest level's long lines)
most reflect next to nothing.
So sites that reflect under the tolerance reflect together,
as the next one met that brings them over it (`along` in `light/`),
and a line ends where less than the tolerance gets through:
the moor's first view took over twice the work without,
no pixel more than two steps of colour off.
Left out instead, they cost less still,
but a thin medium, each site of it under the tolerance, would reflect nothing.
No test holds that: one that simply leaves them out passes them all.
Tried, each measured on the valley's first view and a flight on from it
(`--stroll` with `--by-work` in `native/src/main.rs`),
and not kept:

- tiles half or twice as wide, when the light's were its own: more work either way;
- a cube's bricks lit together by one thread: no less work;
- the tiles forgotten with their sites' light, once none read them:
  made again, a flight lit less than half the bricks in a time;
- a site's rounds ended where it settles, later ones its last:
  a tenth fewer rounds, little less work;
- a site's rounds started from the coarser level's light on what it meets:
  a tenth fewer rounds, no less work;
- a site's rounds ended once their steps fall off as a geometric series,
  its rest added:
  as much faster, and as far off a tightly settled light,
  as settling to three times the tolerance (`Shared::settling`),
  which takes a third less time for twice the error,
  both small next to the model's.

Lighting costs several times what it did
before light read sites' shares and bounces settled site by site:
sites filled in part let light on,
so each is one more that reflects, with rounds of its own.
Of the light alone, about a third is `shown`'s:
the settled light of every neighbour whose pieces are not shown.

Lighting a place first, most is its tiles,
the coarsest level's far around, which no ray reads:
each site met there reflects, so its light is found too, as far around it again.
A line there takes as many steps as the world's heights are sites high
(`top_steps` in `light/`), then the sky's light:
one that rises or falls has left them by then,
and one that keeps level, which never does, ends as far,
so what stands farther off shades nothing along it.
Chosen: the reference's scenes do not tell it from 64 steps.
Tried there, and not kept:

- 64 steps, nearly twice as far in the valley:
  its mountains seen from afar (README's views) took half as much work again or more,
  under a hundredth of their pixels over two steps of colour off, on the far slopes;
- 16 steps: the first view drawn alike, for a tenth less work than at 64;
  not looked at where mountains stand far off, which the steps are for;
- levels past the coarsest, each taking a step as the others do,
  so that no level is unlike the rest: twice the work,
  as a column of level 12's tiles costs thirty times level 8's
  ("Claims cost coarse tiles most" in [todo.md](todo.md)).
  Were coarse tiles as cheap as fine ones, a few a level would do.

Light bounces at most eight times: a site's rounds stop there (`ROUNDS` in `light/`).
Where most of it reflects, spread wide,
it settles in tens of rounds,
each reading the coarsest level's sites a line's length further round,
and a lit view may never finish.
Under clouds tried in [#301](https://github.com/SupraSummus/parallax-world/pull/301),
eight lit a view in seconds, no pixel over two steps of colour off light settled tightly;
four, up to ten off; sixteen took minutes.
Of the reference's scenes, only the cave tells eight from more, a fiftieth darker.

Light adds up, so it is the sum of the sky's, the sun's and the glow's;
kept apart, the sliders could only mix them,
but lighting and keeping that light would cost three times as much.

## Sites, as hosts and light see them

How a site shows to hosts and to light, and what glows, is a prototype too:

- a site's pieces show hosts a voxel where a hash is under the site's alpha
  (`lattice::shows_whole`), as leaves should,
  but measured houses' walls come out ragged;
  light reads their share, of the rays through the voxel
  (`VoxelGrid::light`), so it goes on through sites filled in part
  where it stopped at the first voxel shown:
  a forest or a city at night takes half as long again to light, or more;
  a crown's voxels at the coarse levels come out darker,
  lit under what else is in their sites and those around:
  right or not, no scene of the reference says, its probes all on the ground;
- a site's pieces' colour is one of theirs, picked as often as it is seen,
  not the mean of what its faces see;
  where they show, its medium is not seen;
  a glowing thing's glow is hidden by its own, pieces or medium,
  not by the other;
- the voxels engine and `relight` keep one glow a voxel, its first face's
  (`Light::glow`), though its looks by face:
  a lamp on one side of a coarse voxel glows to every ray, or to none;
- measured things are measured from fine voxels kept by chance,
  so their small lamps come and go;
- a lamp smaller than a site glows from the sites around it,
  its place kept (`around` in `shapes.rs`):
  put on the nearest, further off it was in the ground's layer,
  and a fifth of its light fell on open ground.
  What of it falls on a whole site is lost, as a fill behind a wall is:
  in a room one site wide it glows a third as bright,
  and the white room's floor is darker for it;
  two or three steps a level (`STEPS`), not one,
  would read each level only where its sites are small next to the distance,
  but against the reference they sharpen the wall's shadow
  and leave less light under the roofs and right under the lamp:
  the coarser levels' blur no longer hides how few the directions are.
  Light entering a site by each of the five faces it crosses
  (weighed as `cosines`), not by one,
  did better on the flat scenes with four or five steps,
  but darkened the valley, as its side lines ran into uneven ground,
  did not help the yard, and cost 2.5 to 4 times the time;
  mixing each direction's light into its neighbours' blurred more every level;
- a medium's colour is what it sends on of the light it stops, each time,
  and it stops a channel the more, the darker its colour there (generators.md, "Kinds and things"),
  so a thick layer of it, where light is stopped many times on its way out,
  shows darker than its colour (the moor's mist is nearly white for that);
  shaded, hosts show the colour as it is.
  A thick white one under the sky sends back four fifths of its light, not all,
  and light dies sooner inside it than it should:
  perhaps as a site sends light back only the way it came, as a wall does,
  where a medium scatters it on too. Not looked into;
- things in a site add up as if apart, and alpha has 255 steps,
  so at the coarsest levels a lamp stops nothing;
- generating costs more, lit or not: every covered site is put for light.
