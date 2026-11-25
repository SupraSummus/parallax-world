# Light

Light on the lattices (the crate `light/`) is a prototype,
in `look`'s pictures and native's computed lighting.
The model is in its header (`light/src/lib.rs`),
and how native's engines use it in section 4 of [architecture.md](architecture.md)
("The engines, and rays through the voxels").
Here: what it gets wrong, what was tried, and what it costs.
What is left to do is in [todo.md](todo.md).

## What it gets wrong, and why

Light crosses each site from face to face, along 132 lines (`line`),
and each level takes two steps along a line, a period
(`STEPS`; native's panel sets up to three)
before the coarser one takes over,
or more where the coarser fills what it leaves empty.
So light from D away comes from a level whose sites are about D across,
which keeps what a region stops on average (the contract),
not whether it is closed:
walls thinner than the space they close let light in,
openings smaller than it let none (the door and the window below).

A railway tunnel, under 1 to 4 m of rock where its hill is low,
is lit through it from the sky,
as the reference's tunnel under a thin roof is,
its middle 34 times too bright (with 12 directions, hundreds).

Columns put a whole site where its middle was in a span,
so the coarse levels lost a roof thinner than their spacing
along some directions
(their solid spans cover their sites in part now; the tunnel was not lit anew);
and taking over, a coarser level's light comes to the middle of its site,
what lies between there and where it took over unread.

Two periods a level leave the tunnel's middle twice as bright as it should be,
but light under the roofs and from the lamp several times further off.

The sun comes along the lines from it, one on each of the five faces it lights,
each by its cosine, as the reference's.
Light reads a coarser level at the nearest site and its 12 neighbours (`reads`),
so near a corner where six voxels meet it misses one within its reach,
and light jumps a little there;
every site within reach would leak light through walls one site thick.

## Where it is to go, and what was tried

Where light is to go, a step at a time, each measured on these scenes:
sites hold what they stop along each direction, a share, boxes' too
(light reads a site's share already, as hosts show it);
then fewer lines where they cost most ("Lines face to face", below).

With 12 directions, taking over later or less did not do, each tried against the reference:
only where the coarser level is open some sites ahead,
only the share of the light its sites around have room for,
or after two steps.
Each lit the closed room better,
and left the roof's middle and the room past the yard's door dark, or lit it many times too much:
there no direction of the 12 meets what is lit,
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

With 12 directions, a beam along one entered a voxel by five faces,
the site across one read and `CROSSING` standing for the rest;
the five read as lines of their own let light round walls one site thick,
the four slanted only stopping left the yard several times too dark.
Lines from face to face are what came of it.

## Against the reference

Against the reference (`light/src/reference.rs`, test `against_the_reference`;
the ignored test `maps` draws the scenes from above):
a lamp lights the ground within a sixth;
under a roof along the lattice, light is three quarters of what it should be,
and off it within a third;
a wall's shadow in the sun is within a tenth near the wall, a quarter too dark further off;
past a door from a yard, where only bounced light comes,
a room is twenty times too dark, and the yard half as bright:
the room's light is the yard seen low through the door,
under the lowest of the lines that land on its floor;
with 12 directions the coarser levels' blur brought it there,
about as bright as it should be.

In a closed white room, where light bounces many times,
the light on the floor is a fifth too much
(with 12 directions, a few times too little).

A lamp lights only as far as the coarser levels hold it:
the cheese's crystals stopped at level 3, and its caverns were black
a few metres from them.
Past it they are glowing pieces (`far` in `worlds/cheese/`):
on its walls, measured by hand, light is a third over the reference;
drawn as a medium, a sixth under.
Light reads the two unlike; the contract does not tell them apart.

A site met reflects the light on its one face turned back,
a third of what it shows that way.

Tried: the light on all the faces it shows, each by its share:
no closer to the reference.
And with it a whole voxel touched read as any site met, not lit as the face:
a wall one site thick lit its dark side, and a white room was darker.

It sends back no glow: a lamp's voxels do not light each other's faces,
so a lamp of white voxels is a third dimmer near than the reference's.

A coarse site holding part of a door (as `Far::measured` draws one)
lets light through in every direction alike:
drawn so, the yard's door made the room tens of times too bright.

A window smaller than the coarser levels' sites lets no sky in at all
(they close the room), so a room lit by its windows is black.

## In native, and what it costs

Native's bakers each light what they show with a `Light` of their own:
they share the world, its tiles, and the light found at their sites (`Shared`),
with each other and the world's threads,
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
and two finding the same at once do no harm:
each value is kept once found, by whichever finds it first,
and read with no lock (`Leaving` in `light/`).
Under one lock a column of tiles, and found by a map,
the threads waited for each other, and the first view took a quarter more work;
a lock a site, no less work for one thread than none.
Each Light holds the tiles it read (`Level`), asking the others for each once.
They keep the light of the tiles read lately (`KEPT_SITES`, `Shared::begun`):
fewer hold less, and under what a view reads at once, lighting slows by half.
Forgotten brick by brick, not tile by tile,
a flight held a tenth less and owed twice the bricks.
What a site keeps, and how small, is in "Lines face to face", below.

Native's light sliders light the world anew (`World::relight`),
as slowly as at the start.

Lighting the valley's first view, over two thirds of the world's threads' work
is generating, the tiles rays ask for and those the light reads around them,
and a quarter finding the light.
Of that, most is the rounds past a site's first:
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

## Lines face to face, and what they cost

The light of a site is kept by the face of the next it lands on,
11 lines a face, each face's found only where asked (`Leaving` in `light/`):
in the forest's first view, about 7 of a site's 12.
A face keeps what comes along its lines (`Paths`),
the sites met that reflect, and what it lands with in each round;
132 lines in all, against 12 directions: several times what a site kept.
So it is small (`Packed`, `Reflects`):
a light in a word, 9 bits a channel and an exponent they share (RGB9E5),
and a site met in a word too, its share of 6 bits a channel and 3 of exponent,
as what reflects less than the tolerance is owed to the next;
a face about 260 bytes, a third less than in 16 bits a channel,
3% of a view's pixels one step of colour off, none two,
and the reference's scenes off by a hundredth or two more.

Measured against 12 directions,
on `look`'s pictures 240 pixels wide (the light on one thread, its tiles made as it reads them)
and native's first view of the valley (the voxels engine, 640 by 400):

| | 12 directions | lines |
|---|---|---|
| `look`, forest, lit | 6.6 s, 660 MB | 22 s, 1160 MB |
| `look`, valley, lit | 10.6 s, 770 MB | 34–40 s, 1290 MB |
| `look`, city at night | 3.5 s, 520 MB | 8.3 s |
| native, valley: the threads' work | 9–10 s | 70–87 s |
| native, valley: memory at most | 1.1 GB | 1.9 GB |

`look` lights only the faces its pixels see, which few lines from the sky reach,
and their reflections; native lights every voxel of a brick a ray can meet,
all faces of it light gets to:
in a wood's crowns, every site filled in part has 12 open neighbours, each a face of 11 lines.
There, by the model itself, a site costs 132 lines two steps each, not 12 lines one step.

Tried, and kept:

- what a site is seen as, kept by each Light for the last few thousand faces' lines (`seen`):
  a sixth less time, where most steps read a site read just before;
- each face's rounds and light kept once found, with no lock, no vector, no `Arc`:
  much the same time for one thread, fewer allocations;
- the last round's lines kept to be the light found, not found again: a sixth fewer reads.

Tried, and not kept:

- a line's crossings of a site by its length through it,
  3/4 to 3/2 as its two steps turn (where `CROSSING` is 3/2 for all):
  a lamp smaller than a site glowed dimmer, the strewn blocks stopped too little,
  each a few times further off the reference;
- a site's 12 faces kept in one block, not each when begun: no faster, more held.

## Sites, as hosts and light see them

How a site shows to hosts and to light, and what glows, is a prototype too:

- a site's pieces show hosts a voxel where a hash is under the site's alpha
  (`fcc::shows_whole`), as leaves should,
  but measured houses' walls come out ragged;
  light reads their share, of the rays through the voxel
  (`VoxelGrid::light`; a line through sites' middles meets it
  3/2 as often, `CROSSING` in `fcc/src/ray.rs`),
  so it goes on through sites filled in part
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
  with 12 directions, two or three steps a level, not one,
  would read each level only where its sites are small next to the distance,
  but against the reference they sharpen the wall's shadow
  and leave less light under the roofs and right under the lamp:
  the coarser levels' blur no longer hides how few the directions are.
  Light entering a site by each of the five faces it crosses, not by one,
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
