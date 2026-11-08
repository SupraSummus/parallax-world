"""A glTF scene as voxels of the FCC lattice at level 0, for worlds/src/scene.rs.

    python3 scripts/voxelize.py SCENE.gltf OUT.bin [--scale METRES_PER_UNIT]

A .glb works too. --scale makes a scene bigger, so finer than the lattice's
unit (worldgen/src/units.rs).

Every triangle is sampled on a grid finer than the lattice; each sample
goes to its nearest site and brings the colour of its material there
(base colour texture times factor; samples alpha-masked away are dropped).
Samples are summed per site as they come, so memory goes with the sites.
A site's colour is its samples' mean, and the colours are quantized to a
palette of PALETTE, since material ids are bytes. Only surfaces become
voxels: a closed scene is hollow, which no camera inside sees.

Needs numpy and Pillow. The format is read and documented by scene.rs.
"""

import argparse
import base64
import io
import json
import struct
import sys
from pathlib import Path

import numpy as np
from PIL import Image

# worldgen/src/units.rs: metres per lattice unit.
UNIT_SIZE = 0.125
# Colours a scene may have: material 0 is none.
PALETTE = 255
# Distance between a triangle's samples, lattice units: well under the
# half unit that separates a site from its voxel's faces.
STEP = 0.3

COMPONENTS = {'SCALAR': 1, 'VEC2': 2, 'VEC3': 3, 'VEC4': 4, 'MAT4': 16}
TYPES = {5120: np.int8, 5121: np.uint8, 5122: np.int16, 5123: np.uint16, 5125: np.uint32, 5126: np.float32}


def load(path):
    """The glTF's JSON and a reader of its buffers' bytes by URI (files or data)."""
    path = Path(path)
    if path.suffix == '.glb':
        data = path.read_bytes()
        length = struct.unpack_from('<I', data, 12)[0]
        gltf = json.loads(data[20:20 + length])
        rest = data[20 + length:]
        binary = rest[8:8 + struct.unpack_from('<I', rest, 0)[0]]
    else:
        gltf = json.loads(path.read_text())
        binary = None

    def read(uri):
        if uri is None:
            return binary
        if uri.startswith('data:'):
            return base64.b64decode(uri.split(',', 1)[1])
        return (path.parent / uri).read_bytes()

    return gltf, read


def accessor(gltf, buffers, index):
    a = gltf['accessors'][index]
    view = gltf['bufferViews'][a['bufferView']]
    dtype = np.dtype(TYPES[a['componentType']])
    n = COMPONENTS[a['type']]
    start = view.get('byteOffset', 0) + a.get('byteOffset', 0)
    stride = view.get('byteStride', dtype.itemsize * n)
    raw = np.frombuffer(buffers[view['buffer']], np.uint8, count=stride * (a['count'] - 1) + dtype.itemsize * n, offset=start)
    rows = np.lib.stride_tricks.as_strided(raw, (a['count'], dtype.itemsize * n), (stride, 1))
    values = np.ascontiguousarray(rows).view(dtype).reshape(a['count'], n).astype(np.float64)
    if a.get('normalized'):
        values /= np.iinfo(dtype).max
    return values


def node_matrices(gltf):
    """Each mesh node's matrix to the scene's frame, from its ancestors' TRS or matrices."""
    def local(node):
        if 'matrix' in node:
            return np.array(node['matrix'], np.float64).reshape(4, 4).T
        t = np.eye(4)
        t[:3, 3] = node.get('translation', [0, 0, 0])
        x, y, z, w = node.get('rotation', [0, 0, 0, 1])
        r = np.eye(4)
        r[:3, :3] = [
            [1 - 2 * (y * y + z * z), 2 * (x * y - z * w), 2 * (x * z + y * w)],
            [2 * (x * y + z * w), 1 - 2 * (x * x + z * z), 2 * (y * z - x * w)],
            [2 * (x * z - y * w), 2 * (y * z + x * w), 1 - 2 * (x * x + y * y)],
        ]
        s = np.diag(node.get('scale', [1, 1, 1]) + [1])
        return t @ r @ s

    out = []

    def walk(i, parent):
        node = gltf['nodes'][i]
        m = parent @ local(node)
        if 'mesh' in node:
            out.append((node['mesh'], m))
        for c in node.get('children', []):
            walk(c, m)

    scene = gltf['scenes'][gltf.get('scene', 0)]
    for i in scene['nodes']:
        walk(i, np.eye(4))
    return out


def srgb_to_linear(c):
    return np.where(c <= 0.04045, c / 12.92, ((c + 0.055) / 1.055) ** 2.4)


def linear_to_srgb(c):
    return np.where(c <= 0.0031308, c * 12.92, 1.055 * c ** (1 / 2.4) - 0.055)


def textures(gltf, read):
    """Base colour of each material: (RGBA texture as linear floats or None, factor, alpha cutoff or None, texcoord set)."""
    images = {}

    def image(i):
        if i not in images:
            img = gltf['images'][i]
            if 'uri' in img:
                data = read(img['uri'])
            else:
                view = gltf['bufferViews'][img['bufferView']]
                data = read(gltf['buffers'][view['buffer']].get('uri'))[view.get('byteOffset', 0):][:view['byteLength']]
            rgba = np.asarray(Image.open(io.BytesIO(data)).convert('RGBA'), np.float32) / 255
            rgba[..., :3] = srgb_to_linear(rgba[..., :3])
            images[i] = rgba
        return images[i]

    out = []
    for m in gltf.get('materials', []):
        pbr = m.get('pbrMetallicRoughness', {})
        factor = np.array(pbr.get('baseColorFactor', [1, 1, 1, 1]), np.float32)
        tex = pbr.get('baseColorTexture')
        rgba = image(gltf['textures'][tex['index']]['source']) if tex else None
        cutoff = m.get('alphaCutoff', 0.5) if m.get('alphaMode') == 'MASK' else None
        out.append((rgba, factor, cutoff, tex.get('texCoord', 0) if tex else 0))
    return out


def triangle_samples(n):
    """Barycentric coordinates of a grid n steps along each edge."""
    i, j = np.meshgrid(np.arange(n + 1), np.arange(n + 1), indexing='ij')
    keep = i + j <= n
    a, b = i[keep] / n, j[keep] / n
    return np.stack([1 - a - b, a, b], axis=1)


def sample(points, uvs, material, materials):
    """Samples of triangles (T, 3, 3) with their uvs (T, 3, 2), in chunks: positions and linear RGB, alpha-masked ones dropped."""
    rgba, factor, cutoff, _ = materials[material] if material is not None else (None, np.ones(4, np.float32), None, 0)
    edges = np.linalg.norm(points - np.roll(points, 1, axis=1), axis=2).max(axis=1)
    steps = np.maximum(1, np.ceil(edges / STEP)).astype(int)
    for n in np.unique(steps):
        tri = steps == n
        bary = triangle_samples(n)
        # Chunks of at most a few million samples.
        per = max(1, 4_000_000 // len(bary))
        for start in range(0, tri.sum(), per):
            p = np.einsum('sk,tkd->tsd', bary, points[tri][start:start + per]).reshape(-1, 3)
            colour = np.broadcast_to(factor, (len(p), 4)).copy()
            if rgba is not None and uvs is not None:
                uv = np.einsum('sk,tkd->tsd', bary, uvs[tri][start:start + per]).reshape(-1, 2)
                h, w = rgba.shape[:2]
                x = (np.floor(uv[:, 0] * w).astype(np.int64)) % w
                y = (np.floor(uv[:, 1] * h).astype(np.int64)) % h
                colour *= rgba[y, x]
            keep = colour[:, 3] >= cutoff if cutoff is not None else slice(None)
            yield p[keep], colour[keep, :3]


class Sites:
    """Sites with the sum of their samples' colours and their count, summed as samples come."""
    # Keys: x, y and z, each 21 bits, offset to be positive.
    BITS, OFFSET = 21, 1 << 20

    def __init__(self):
        self.parts = []
        self.size = 0

    def add(self, sites, colours):
        b, o = self.BITS, self.OFFSET
        key = (((sites[:, 0] + o) << b) + sites[:, 1] + o << b) + sites[:, 2] + o
        self.parts.append((key, colours, np.ones(len(key), np.int64)))
        self.size += len(key)
        if self.size > 20_000_000:
            self.compact()

    def compact(self):
        key, colour, count = (np.concatenate(a) for a in zip(*self.parts))
        unique, inverse = np.unique(key, return_inverse=True)
        sums = np.stack([np.bincount(inverse, colour[:, i], len(unique)) for i in range(3)], axis=1)
        self.parts = [(unique, sums, np.bincount(inverse, count, len(unique)))]
        self.size = len(unique)

    def result(self):
        """The sites (S, 3) and their mean colours."""
        self.compact()
        key, sums, count = self.parts[0]
        b, o, mask = self.BITS, self.OFFSET, (1 << self.BITS) - 1
        sites = np.stack([(key >> 2 * b) - o, (key >> b & mask) - o, (key & mask) - o], axis=1)
        return sites, sums / count[:, None]


def nearest_sites(p):
    """The FCC sites (x + y + z even) nearest to points p, lattice units."""
    r = np.rint(p)
    odd = (r.sum(axis=1).astype(np.int64) & 1) == 1
    # An odd point is moved along the axis it is furthest from, towards p.
    off = p[odd] - r[odd]
    axis = np.abs(off).argmax(axis=1)
    rows = np.nonzero(odd)[0]
    r[rows, axis] += np.where(off[np.arange(len(rows)), axis] >= 0, 1, -1)
    return r.astype(np.int64)


def main():
    parser = argparse.ArgumentParser(description=__doc__.split('\n\n')[0])
    parser.add_argument('scene')
    parser.add_argument('out')
    parser.add_argument('--scale', type=float, default=1.0, help="metres per the scene's unit, after its nodes' transforms")
    args = parser.parse_args()

    gltf, read = load(args.scene)
    buffers = [read(b.get('uri')) for b in gltf['buffers']]
    materials = textures(gltf, read)

    sites, samples = Sites(), 0
    for mesh, matrix in node_matrices(gltf):
        for prim in gltf['meshes'][mesh]['primitives']:
            if prim.get('mode', 4) != 4:
                continue
            pos = accessor(gltf, buffers, prim['attributes']['POSITION'])
            pos = (pos @ matrix[:3, :3].T + matrix[:3, 3]) * args.scale / UNIT_SIZE
            material = prim.get('material')
            set_ = materials[material][3] if material is not None else 0
            uv_key = f'TEXCOORD_{set_}'
            uv = accessor(gltf, buffers, prim['attributes'][uv_key]) if uv_key in prim['attributes'] else None
            index = accessor(gltf, buffers, prim['indices']).astype(np.int64).reshape(-1, 3) if 'indices' in prim else np.arange(len(pos)).reshape(-1, 3)
            for p, c in sample(pos[index], uv[index] if uv is not None else None, material, materials):
                sites.add(nearest_sites(p), c)
                samples += len(p)
    print(f'{samples} samples', file=sys.stderr)

    sites, mean = sites.result()
    lo = sites.min(axis=0)
    # Keep the lattice: the origin moves by an even amount on each axis.
    lo -= lo & 1
    sites -= lo
    n = sites.max(axis=0) + 1
    # An even width: each row of the box (along x) has n[0] / 2 sites.
    n[0] += n[0] & 1
    srgb = (np.clip(linear_to_srgb(mean), 0, 1) * 255 + 0.5).astype(np.uint8)
    print(f'{len(sites)} sites in {n.tolist()}', file=sys.stderr)

    # Median cut over the sites' colours, a pixel each.
    quantized = Image.fromarray(srgb.reshape(1, -1, 3)).quantize(PALETTE, Image.Quantize.MEDIANCUT)
    ids = np.asarray(quantized, np.uint8).reshape(-1).astype(np.int64) + 1
    palette = np.array(quantized.getpalette()[:3 * (ids.max())], np.uint8)

    out = bytearray(b'FCC0')
    out += struct.pack('<I', len(palette) // 3) + palette.tobytes()
    out += struct.pack('<6i', *[int(v) for v in lo], *[int(v) for v in n])

    def varint(v):
        while v >= 0x80:
            out.append(v & 0x7f | 0x80)
            v >>= 7
        out.append(v)

    # Sites in order z, y, x, as runs: a run of empty sites, then of full ones, each a varint, the full ones' ids after.
    rank = (sites[:, 2] * n[1] + sites[:, 1]) * (n[0] // 2) + sites[:, 0] // 2
    order = np.argsort(rank)
    rank, ids = rank[order], ids[order].astype(np.uint8)
    starts = np.flatnonzero(np.diff(rank, prepend=-2) != 1)
    ends = np.append(starts[1:], len(rank))
    at = 0
    for start, end in zip(starts, ends):
        varint(int(rank[start] - at))
        varint(int(end - start))
        out += ids[start:end].tobytes()
        at = rank[end - 1] + 1
    # The last run has no full sites: the end.
    varint(int(n[0] // 2 * n[1] * n[2] - at))
    varint(0)
    Path(args.out).write_bytes(out)
    print(f'{len(palette) // 3} colours, {len(out)} bytes', file=sys.stderr)


if __name__ == '__main__':
    main()
