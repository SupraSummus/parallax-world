# Light

Light on the lattices (`worldgen/src/light.rs`) is a prototype,
in `look`'s pictures and native's computed lighting;
the web does not light its tiles.
The model is in that file's header,
and how native's engines use it in section 4 of [architecture.md](architecture.md)
("Native: another engine").
Here: what it gets wrong, what was tried, and what it costs.
What is left to do is in [todo.md](todo.md).

## What it gets wrong, and why

Light comes along the 12 neighbours' directions only,
and each level takes one step (`STEPS`; native's panel sets more)
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
its middle hundreds of times too bright.

Columns put a whole site where its middle was in a span,
so the coarse levels lost a roof thinner than their spacing
along some directions
(their solid spans cover their sites in part now; the tunnel was not lit anew);
and taking over, a coarser level's light comes to the middle of its site,
what lies between there and where it took over unread.

Three steps a level leave the tunnel ten times too bright, not hundreds,
but the lamp several times further off.

The sun is one of the 12 directions.
Light reads a coarser level at the nearest site and its 12 neighbours (`reads`),
so near a corner where six voxels meet it misses one within its reach,
and light jumps a little there;
every site within reach would leak light through walls one site thick.

## Where it is to go, and what was tried

Where light is to go, a step at a time, each measured on these scenes:
sites hold what they stop along each direction, a share, boxes' too
(light reads a site's share already, not the coin hosts are shown by);
then coarser levels take more directions ("Ideas" in [todo.md](todo.md)).

Taking over later or less did not do, each tried against the reference:
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

A site whose light is none in two rounds running is found dark (`Light::change`),
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

Against the reference (`light/reference.rs`, test `against_the_reference`;
the ignored test `maps` draws the scenes from above):
a lamp lights the ground in four lobes, along the upward directions,
and three times too little right under it;
under a roof turned off the lattice, light is off by up to 1.7 times;
along the lattice, its middle gets a third of what it should:
from there no direction meets the lit ground past its edge;
a wall's shadow in the sun is short and blotchy,
as the levels that carry it get coarser;
past a door from a yard, where only bounced light comes,
a room is about as bright as it should be, and the yard a third as bright.

In a closed white room or the cheese, where light bounces many times,
the light on the floor is a few times too little,
as little as reaches it from the lamp.

A site met reflects the light on its one face turned back,
a third of what it shows that way.

Tried: the light on all the faces it shows, each by its share:
no closer to the reference.
And with it a whole voxel touched read as any site met, not lit as the face:
a wall one site thick lit its dark side, and a white room was darker.

A whole voxel touching a face is taken to be lit as the face is
(`weighed` in `light.rs`), as a flat floor's are;
where it is not, at a wall's edge or a corner, the face is lit wrong.

It sends back no glow: a lamp's voxels do not light each other's faces,
so a lamp of white voxels is a third dimmer near than the reference's.

A coarse site holding part of a door (as `Far::measured` draws one)
lets light through in every direction alike:
drawn so, the yard's door made the room tens of times too bright.

A window smaller than the coarser levels' sites lets no sky in at all
(they close the room), so a room lit by its windows is black.

## In native, and what it costs

Native's bakers each light what they show with a `Light` of their own
(a generator is not `Send`):
they share blocks (`Shared`) with each other and the world's threads,
not the world's tiles, which keep of a site only what is drawn:
what is lit is generated twice, as a tile and as blocks,
till hosts read sites as light does
("Cells hold what they look like" in [generators.md](generators.md));
the tiles' own (`TILE`, `tile_box`, `make_tiles`) are still in `light.rs`,
which no longer makes them;
but each finds light again where another found it
(all but the coarsest level's: `Shared`'s `top`),
and makes a generator, whose plans' caches take tens of MB;
each of the world's threads makes two, its own and its Light's.
A site of a block costs a few times one of a tile at the finest levels
(the cheese's holes, the valley's plans, asked again per block),
so a tile read all round its edges costs about as much in blocks.

Native's light sliders light the world anew (`World::relight`),
as slowly as at the start.

Lighting costs several times what it did
before light read sites' shares and bounces settled site by site:
sites filled in part let light on,
so each is one more that reflects, with rounds of its own.
Of the light alone, about a third is `shown`'s:
the settled light of every neighbour whose pieces are not shown.

Lighting a place first, most is its blocks,
the coarsest level's far around (`TOP_STEPS` sites along every direction).
`look`'s "lit in" overstates it: a picture's rows cross the world,
and each forgets what the last found (`KEPT_SITES`),
where bakers light what is side by side.

Light adds up, so it is the sum of the sky's, the sun's and the glow's;
kept apart, the sliders could only mix them,
but lighting and keeping that light would cost three times as much.

## Sites, as hosts and light see them

How a site shows to hosts and to light, and what glows, is a prototype too:

- a site's pieces show hosts a voxel where a hash is under their alpha
  (`VoxelGrid::show`), as leaves should,
  but measured houses' walls come out ragged;
  light reads their share, of the rays through the voxel
  (`VoxelGrid::light`; a line through sites' middles meets it
  3/2 as often, `CROSSING` in `picture.rs`),
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
- things in a site add up as if apart, and alpha has 255 steps,
  so at the coarsest levels a lamp stops nothing;
- generating costs more, lit or not: every covered site is put for light.
