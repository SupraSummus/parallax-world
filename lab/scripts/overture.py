"""Real places as the lab's label maps: Overture Maps (OpenStreetMap's
buildings, roads, land use and water, and land cover from satellites) for
towns and villages of a country, each a square around its centre drawn at
a metre a cell in towns.rs's eight labels. Read straight from Overture's
public bucket, only the row groups whose boxes meet a place's.

    python3 -m venv venv && venv/bin/pip install pyarrow shapely rasterio numpy
    venv/bin/python lab/scripts/overture.py [--country PL] [--towns 40] [--villages 40] [--side 1536]

Writes lab/out/real/: per place its map and its lines, main roads and
water as the map shows them, 255 elsewhere (each u32 width, u32 height,
then a byte a cell, rows from the south, x east); and places.tsv.
real.rs reads them.
The row groups' boxes are kept in index-*.json, so a second run reads only
what it lacks. Data © OpenStreetMap contributors (ODbL), Overture Maps.
"""

import argparse
import json
import math
import os
import random
import struct
import sys

import numpy as np
import pyarrow.compute as pc
import pyarrow.fs as pafs
import pyarrow.parquet as pq
import rasterio.features
import shapely
from rasterio.transform import Affine

RELEASE = "2026-10-21.0"
BUCKET = f"overturemaps-us-west-2/release/{RELEASE}/"
OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "out", "real")

# towns.rs's labels.
ROOF, WALL, STREET, PAVING, WATER, CANOPY, GROUND, FIELDS = range(8)

# Roads by class: label and width in metres.
ROADS = {
    "motorway": (STREET, 14), "trunk": (STREET, 11), "primary": (STREET, 9), "secondary": (STREET, 8),
    "tertiary": (STREET, 7), "residential": (STREET, 6), "unclassified": (STREET, 5), "living_street": (STREET, 5),
    "service": (STREET, 4), "pedestrian": (PAVING, 5), "footway": (PAVING, 2), "steps": (PAVING, 2),
    "cycleway": (PAVING, 2), "track": (GROUND, 3), "path": (GROUND, 1.5), "bridleway": (GROUND, 2),
}
# Land use by class, and land cover by subtype; what is not here stays ground.
LAND_USE = {
    "farmland": FIELDS, "farmyard": GROUND, "orchard": CANOPY, "vineyard": FIELDS, "allotments": FIELDS,
    "forest": CANOPY, "wood": CANOPY, "plantation": CANOPY, "meadow": GROUND, "grass": GROUND,
    "pedestrian": PAVING, "parking": PAVING, "plaza": PAVING, "marketplace": PAVING, "railway": GROUND,
    "residential": GROUND, "commercial": GROUND, "retail": GROUND, "industrial": GROUND, "cemetery": GROUND,
}
# The roads given as lines in a place's plan (`.lines.bin`): those between places; the streets within them are left to models.
MAIN = {"motorway", "trunk", "primary", "secondary", "tertiary", "unclassified"}
NONE = 255
LAND_COVER = {"forest": CANOPY, "crop": FIELDS, "wetland": GROUND, "shrub": GROUND, "grass": GROUND, "urban": GROUND}


def s3():
    proxy = os.environ.get("HTTPS_PROXY") or os.environ.get("https_proxy")
    return pafs.S3FileSystem(anonymous=True, region="us-west-2", **({"proxy_options": proxy} if proxy else {}))


def index(fs, theme):
    """Per file of a theme, per row group, its bounding box (from the bbox columns' statistics); cached."""
    path = os.path.join(OUT, f"index-{theme.replace('/', '-').replace('=', '_')}.json")
    if os.path.exists(path):
        return json.load(open(path))
    files = [f.path for f in fs.get_file_info(pafs.FileSelector(BUCKET + theme)) if f.path.endswith(".parquet")]
    out = []
    for i, f in enumerate(files):
        meta = pq.ParquetFile(f, filesystem=fs).metadata
        names = [meta.schema.column(c).path for c in range(meta.num_columns)]
        col = {k: names.index(f"bbox.{k}") for k in ("xmin", "xmax", "ymin", "ymax")}
        for g in range(meta.num_row_groups):
            rg = meta.row_group(g)
            s = {k: rg.column(c).statistics for k, c in col.items()}
            out.append([f, g, s["xmin"].min, s["xmax"].max, s["ymin"].min, s["ymax"].max])
        print(f"  index {theme}: {i + 1}/{len(files)}", file=sys.stderr, end="\r")
    print(file=sys.stderr)
    json.dump(out, open(path, "w"))
    return out


def gather(fs, theme, columns, places):
    """Each place's features of a theme: reads each row group meeting some place's box once."""
    found = {p["id"]: [] for p in places}
    wanted = {}
    for f, g, x0, x1, y0, y1 in index(fs, theme):
        for p in places:
            b = p["box"]
            if x0 < b[2] and x1 > b[0] and y0 < b[3] and y1 > b[1]:
                wanted.setdefault((f, g), []).append(p)
    opened = {}
    for i, ((f, g), ps) in enumerate(sorted(wanted.items())):
        if f not in opened:
            opened = {f: pq.ParquetFile(f, filesystem=fs)}
        t = opened[f].read_row_group(g, columns=columns + ["bbox"])
        for p in ps:
            b = p["box"]
            bb = t["bbox"]
            m = pc.and_(
                pc.and_(pc.less(pc.struct_field(bb, "xmin"), b[2]), pc.greater(pc.struct_field(bb, "xmax"), b[0])),
                pc.and_(pc.less(pc.struct_field(bb, "ymin"), b[3]), pc.greater(pc.struct_field(bb, "ymax"), b[1])),
            )
            found[p["id"]].append(t.filter(m))
        print(f"  {theme}: {i + 1}/{len(wanted)} row groups", file=sys.stderr, end="\r")
    print(file=sys.stderr)
    return found


def project(geom, p):
    """Lon/lat to metres east and north of the place's centre, plus half its side."""
    lon0, lat0, half = p["lon"], p["lat"], p["side"] / 2
    kx = 111_320 * math.cos(math.radians(lat0))
    return shapely.transform(geom, lambda c: np.column_stack([(c[:, 0] - lon0) * kx + half, (c[:, 1] - lat0) * 110_540 + half]))


def draw(p, layers):
    """
    The place's labels, painted in order: land cover, land use, water,
    paths, roads, buildings; and its lines: water and main roads where the
    labels show them, NONE elsewhere.
    """
    side = p["side"]
    grid = np.full((side, side), GROUND, dtype=np.uint8)
    lines = np.full((side, side), NONE, dtype=np.uint8)
    # Rows from the south: row y is y metres north of the square's edge.
    transform = Affine(1, 0, 0, 0, 1, 0)

    def paint(shapes, out=grid):
        shapes = [(g, v) for g, v in shapes if g is not None and not g.is_empty]
        if shapes:
            rasterio.features.rasterize(shapes, out=out, transform=transform)

    def geoms(tables, key):
        for t in tables:
            gs = shapely.from_wkb(t["geometry"].to_numpy(zero_copy_only=False))
            ks = t[key].to_pylist() if key else [None] * len(gs)
            for g, k in zip(gs, ks):
                yield project(g, p), k

    paint((g, LAND_COVER[k]) for g, k in geoms(layers["land_cover"], "subtype") if k in LAND_COVER)
    paint((g, LAND_USE[k]) for g, k in geoms(layers["land_use"], "class") if k in LAND_USE)
    water = [(g if g.geom_type.endswith("Polygon") else g.buffer(2), WATER) for g, _ in geoms(layers["water"], "subtype")]
    paint(water)
    roads = [(g, k, ROADS[k]) for g, k in geoms(layers["segment"], "class") if k in ROADS]
    for wanted in (GROUND, PAVING, STREET):
        paint((g.buffer(w / 2, cap_style="flat"), l) for g, _, (l, w) in roads if l == wanted)
    paint((g, ROOF) for g, _ in geoms(layers["building"], None))
    paint(water, lines)
    paint(((g.buffer(w / 2, cap_style="flat"), l) for g, k, (l, w) in roads if k in MAIN), lines)
    lines[lines != grid] = NONE
    return grid, lines


def places(fs, country, towns, villages, side, seed):
    """Towns and villages of the country (Overture's localities), drawn at random."""
    t = pq.read_table(
        BUCKET + "theme=divisions/type=division",
        filesystem=fs,
        columns=["id", "names", "class", "population", "bbox"],
        filters=[("country", "=", country), ("subtype", "=", "locality")],
    ).to_pylist()
    rng = random.Random(seed)
    out = []
    for cls, k in (("town", towns), ("village", villages)):
        pick = sorted((r for r in t if r["class"] == cls), key=lambda r: r["id"])
        for r in rng.sample(pick, min(k, len(pick))):
            lon, lat = (r["bbox"]["xmin"] + r["bbox"]["xmax"]) / 2, (r["bbox"]["ymin"] + r["bbox"]["ymax"]) / 2
            dlon, dlat = side / 2 / (111_320 * math.cos(math.radians(lat))), side / 2 / 110_540
            out.append({
                "id": f"{country.lower()}-{cls}-{len(out):03d}",
                "name": r["names"]["primary"], "class": cls, "population": r["population"] or 0,
                "lon": lon, "lat": lat, "side": side, "box": [lon - dlon, lat - dlat, lon + dlon, lat + dlat],
            })
    return out


def main():
    a = argparse.ArgumentParser()
    a.add_argument("--country", default="PL")
    a.add_argument("--towns", type=int, default=40)
    a.add_argument("--villages", type=int, default=40)
    a.add_argument("--side", type=int, default=1536)
    a.add_argument("--seed", type=int, default=1)
    args = a.parse_args()
    os.makedirs(OUT, exist_ok=True)
    fs = s3()
    ps = places(fs, args.country, args.towns, args.villages, args.side, args.seed)
    todo = [p for p in ps if not os.path.exists(os.path.join(OUT, p["id"] + ".lines.bin"))]
    print(f"{len(ps)} places, {len(todo)} to draw", file=sys.stderr)
    themes = {
        "land_cover": ("theme=base/type=land_cover", ["geometry", "subtype"]),
        "land_use": ("theme=base/type=land_use", ["geometry", "class"]),
        "water": ("theme=base/type=water", ["geometry", "subtype"]),
        "segment": ("theme=transportation/type=segment", ["geometry", "class"]),
        "building": ("theme=buildings/type=building", ["geometry"]),
    }
    if todo:
        layers = {k: gather(fs, theme, cols, todo) for k, (theme, cols) in themes.items()}
        for p in todo:
            grid, lines = draw(p, {k: layers[k][p["id"]] for k in themes})
            for name, a in ((".bin", grid), (".lines.bin", lines)):
                with open(os.path.join(OUT, p["id"] + name), "wb") as f:
                    f.write(struct.pack("<II", a.shape[1], a.shape[0]))
                    f.write(a.tobytes())
    with open(os.path.join(OUT, "places.tsv"), "w") as f:
        f.write("id\tname\tclass\tpopulation\tlon\tlat\tside\n")
        for p in ps:
            f.write(f"{p['id']}\t{p['name']}\t{p['class']}\t{p['population']}\t{p['lon']:.6f}\t{p['lat']:.6f}\t{p['side']}\n")


if __name__ == "__main__":
    main()
