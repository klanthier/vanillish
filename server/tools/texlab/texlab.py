#!/usr/bin/env python3
"""texlab — upgrade a zone's textures from YOUR OWN 1.12.1 Data folder, and compare.

    texlab.py zone    --data DATA --extract BIN --map Kalimdor --tiles 39-40,32-33 --out WORK
    texlab.py upscale --work WORK --esrgan BIN [--models DIR] [--limit N]
    texlab.py compare --work WORK            # writes WORK/compare.html (and opens it on a Mac)

`zone` reads the zone's terrain tiles (ADT) and every building (WMO) and prop (M2) placed on them,
lists the textures they use, and decodes each one to PNG with benilla-extract. `upscale` makes two
2x versions of each with Real-ESRGAN on the GPU:

  faithful   realesrgan-x4plus-anime, brought down to 2x: keeps the hand-painted flat colour areas
             and edges, removes the blocky compression, adds no new detail;
  detailed   realesrgan-x4plus (the general model), brought down to 2x: adds fine grain to stone,
             bark and dirt — more "modern", and further from the original.

Tiling textures (terrain, buildings, wrapping props) are padded with their own opposite edges before
upscaling so the result still tiles without seams; alpha is upscaled on its own and cut-outs (1-bit
alpha, leaves, fences) get their hard edge back. Colours are pulled back to the original's per
channel, so the palette stays the 2004 one.

Everything stays on this machine. The files are Blizzard's and the upscaled ones derive from them:
for your own use only — never share them or put them where others download from.
Needs: python3 + Pillow (pip3 install --user pillow).
"""

import argparse
import html
import json
import os
import struct
import subprocess
import sys
import tempfile
from pathlib import Path

# ---- the game's files ---------------------------------------------------------------------------


def chunks(b):
    """Top-level chunks of an ADT/WMO: (id as read in tools, i.e. reversed on disk, payload)."""
    i = 0
    while i + 8 <= len(b):
        cid = b[i:i + 4][::-1].decode("latin-1")
        size = struct.unpack_from("<I", b, i + 4)[0]
        yield cid, b[i + 8:i + 8 + size]
        i += 8 + size


def strings(payload):
    return [s.decode("latin-1") for s in payload.split(b"\0") if s]


def adt_refs(b):
    """(textures, m2 names, wmo names) of one terrain tile."""
    tex, m2, wmo = [], [], []
    for cid, p in chunks(b):
        if cid == "MTEX":
            tex = strings(p)
        elif cid == "MMDX":
            m2 = strings(p)
        elif cid == "MWMO":
            wmo = strings(p)
    return tex, m2, wmo


def wmo_textures(b):
    for cid, p in chunks(b):
        if cid == "MOTX":
            return strings(p)
    return []


def m2_textures(b):
    """(filename, wraps) of a vanilla M2's hardcoded textures (header array at 0x5C)."""
    if len(b) < 0x64 or b[:4] != b"MD20":
        return []
    n, ofs = struct.unpack_from("<II", b, 0x5C)
    out = []
    for k in range(min(n, 64)):
        base = ofs + k * 16
        if base + 16 > len(b):
            break
        ttype, flags, flen, fofs = struct.unpack_from("<IIII", b, base)
        if ttype != 0 or flen == 0:
            continue  # creature skins, hair, capes: chosen at run time, not named here
        name = b[fofs:fofs + flen].split(b"\0")[0].decode("latin-1")
        if name:
            out.append((name, bool(flags & 3)))
    return out


def blp_header(b):
    """Format facts the re-encoder will need: compression, alpha depth, size, mips."""
    if b[:4] != b"BLP2" or len(b) < 20:
        return {}
    _type, comp, alpha_depth, alpha_type, has_mips = struct.unpack_from("<IBBBB", b, 4)
    w, h = struct.unpack_from("<II", b, 12)
    return {
        "compression": {1: "palette", 2: "dxt", 3: "argb"}.get(comp, comp),
        "alpha_depth": alpha_depth,
        "alpha_type": alpha_type,
        "mips": bool(has_mips),
        "w": w,
        "h": h,
    }


class Archive:
    """The patch chain, through benilla-extract (later archives override earlier ones)."""

    def __init__(self, extract_bin, data):
        self.bin, self.data = extract_bin, data

    def raw(self, internal):
        with tempfile.NamedTemporaryFile(delete=False) as t:
            path = t.name
        try:
            r = subprocess.run([self.bin, self.data, "extract", internal, path],
                               capture_output=True)
            if r.returncode != 0:
                return None
            return Path(path).read_bytes()
        finally:
            os.unlink(path)

    def png(self, internal, out):
        r = subprocess.run([self.bin, self.data, "blp", internal, str(out)], capture_output=True)
        return r.returncode == 0


def category(path):
    p = path.lower()
    if p.startswith("tileset\\"):
        return "terrain"
    if p.startswith("dungeons\\textures") or p.startswith("world\\wmo"):
        return "buildings"
    if "tree" in p or "bush" in p or "cactus" in p or "leaf" in p or "leaves" in p:
        return "plants"
    return "props"


def cmd_zone(a):
    arc = Archive(a.extract, a.data)
    x0, x1, y0, y1 = parse_tiles(a.tiles)
    work = Path(a.out)
    (work / "orig").mkdir(parents=True, exist_ok=True)
    textures = {}  # internal path (lower) -> entry

    def add(name, kind, wrap):
        key = name.replace("/", "\\").lower()
        if key.startswith("interface\\") or key.startswith("xtextures\\"):
            return  # the interface and liquids are not this lab's business
        e = textures.setdefault(key, {"path": name.replace("/", "\\"), "wrap": False, "from": set()})
        e["wrap"] = e["wrap"] or wrap
        e["from"].add(kind)

    seen_m2, seen_wmo = set(), set()
    for x in range(x0, x1 + 1):
        for y in range(y0, y1 + 1):
            adt = arc.raw(f"World\\Maps\\{a.map}\\{a.map}_{x}_{y}.adt")
            if adt is None:
                print(f"  tile {x}_{y}: not in the archives", file=sys.stderr)
                continue
            tex, m2s, wmos = adt_refs(adt)
            print(f"  tile {x}_{y}: {len(tex)} ground textures, {len(m2s)} props, {len(wmos)} buildings")
            for t in tex:
                add(t, "terrain", True)
            for m in m2s:
                m2 = m.replace("/", "\\")
                if m2.lower().endswith((".mdx", ".mdl")):
                    m2 = m2[:-4] + ".m2"
                if m2.lower() in seen_m2:
                    continue
                seen_m2.add(m2.lower())
                b = arc.raw(m2)
                for name, wrap in m2_textures(b or b""):
                    add(name, "prop", wrap)
            for w in wmos:
                if w.lower() in seen_wmo:
                    continue
                seen_wmo.add(w.lower())
                b = arc.raw(w)
                for name in wmo_textures(b or b""):
                    add(name, "building", True)

    manifest = []
    for i, (key, e) in enumerate(sorted(textures.items())):
        stem = f"{i:04d}"
        raw = arc.raw(e["path"])
        if raw is None:
            continue
        if not arc.png(e["path"], work / "orig" / f"{stem}.png"):
            continue
        manifest.append({
            "id": stem,
            "path": e["path"],
            "wrap": e["wrap"],
            "category": category(e["path"]),
            "blp": blp_header(raw),
        })
    (work / "manifest.json").write_text(json.dumps(manifest, indent=1))
    cats = {}
    for m in manifest:
        cats[m["category"]] = cats.get(m["category"], 0) + 1
    print(f"{len(manifest)} textures in {work}/orig: " + ", ".join(f"{v} {k}" for k, v in sorted(cats.items())))


def parse_tiles(s):
    xs, ys = s.split(",")

    def rng(t):
        lo, _, hi = t.partition("-")
        return int(lo), int(hi or lo)

    (x0, x1), (y0, y1) = rng(xs), rng(ys)
    return x0, x1, y0, y1


# ---- upscaling ----------------------------------------------------------------------------------

PAD = 16  # source pixels of wrapped border around a tiling texture before upscaling


def cmd_upscale(a):
    from PIL import Image, ImageStat

    work = Path(a.work)
    manifest = json.loads((work / "manifest.json").read_text())
    if a.limit:
        manifest = manifest[: a.limit]
    models = a.models or str(Path(a.esrgan).parent / "models")
    variants = {"faithful": "realesrgan-x4plus-anime", "detailed": "realesrgan-x4plus"}
    for v in variants:
        (work / v).mkdir(exist_ok=True)
    tmp = work / "tmp"
    tmp.mkdir(exist_ok=True)
    for n, m in enumerate(manifest, 1):
        src = Image.open(work / "orig" / f"{m['id']}.png").convert("RGBA")
        w, h = src.size
        rgb, alpha = src.convert("RGB"), src.getchannel("A")
        pad = PAD if m["wrap"] and w >= 2 * PAD and h >= 2 * PAD else 0
        padded = wrap_pad(rgb, pad) if pad else rgb
        padded.save(tmp / "in.png")
        for v, model in variants.items():
            out = work / v / f"{m['id']}.png"
            if out.exists() and not a.force:
                continue
            r = subprocess.run([a.esrgan, "-i", str(tmp / "in.png"), "-o", str(tmp / "out.png"),
                                "-n", model, "-s", "4", "-m", models], capture_output=True, text=True)
            if r.returncode != 0 or not (tmp / "out.png").exists():
                print(f"  {m['path']}: {v} failed: {r.stderr.strip()[-200:]}", file=sys.stderr)
                continue
            big = Image.open(tmp / "out.png").convert("RGB")
            if pad:
                big = big.crop((4 * pad, 4 * pad, 4 * (pad + w), 4 * (pad + h)))
            up = big.resize((2 * w, 2 * h), Image.LANCZOS)
            up = match_colours(up, rgb, ImageStat)
            a2 = upscale_alpha(alpha, m, Image)
            up.putalpha(a2)
            up.save(out)
            (tmp / "out.png").unlink()
        print(f"  [{n}/{len(manifest)}] {m['path']}")
    print(f"upscaled into {work}/faithful and {work}/detailed")


def wrap_pad(img, pad):
    """The image with `pad` pixels of its opposite edges around it, as a tiling sampler sees it."""
    from PIL import Image

    w, h = img.size
    big = Image.new(img.mode, (w + 2 * pad, h + 2 * pad))
    for dx in (-1, 0, 1):
        for dy in (-1, 0, 1):
            big.paste(img, (pad + dx * w, pad + dy * h))
    return big


def match_colours(up, orig, ImageStat):
    """Pull each channel's mean and spread back to the original's: the palette stays 2004's."""
    s_up, s_or = ImageStat.Stat(up), ImageStat.Stat(orig)
    bands = []
    for c, band in enumerate(up.split()):
        mu, so = s_up.mean[c], max(s_up.stddev[c], 1e-3)
        mo, sd = s_or.mean[c], s_or.stddev[c]
        k = sd / so
        bands.append(band.point(lambda v, mu=mu, mo=mo, k=k: max(0, min(255, int((v - mu) * k + mo + 0.5)))))
    from PIL import Image

    return Image.merge("RGB", bands)


def upscale_alpha(alpha, m, Image):
    w, h = alpha.size
    a2 = alpha.resize((2 * w, 2 * h), Image.BICUBIC)
    if m.get("blp", {}).get("alpha_depth") == 1 or set(alpha.getdata()) <= {0, 255}:
        a2 = a2.point(lambda v: 255 if v >= 128 else 0)  # a cut-out keeps a hard edge
    return a2


# ---- comparing ----------------------------------------------------------------------------------


def cmd_compare(a):
    work = Path(a.work)
    manifest = json.loads((work / "manifest.json").read_text())
    order = {"terrain": 0, "buildings": 1, "plants": 2, "props": 3}
    manifest.sort(key=lambda m: (order.get(m["category"], 9), m["path"].lower()))
    rows = []
    for m in manifest:
        if not (work / "faithful" / f"{m['id']}.png").exists():
            continue
        name = html.escape(m["path"])
        tile = "tile" if m["wrap"] else ""
        rows.append(f"""
<section class="{m['category']}">
 <h3>{name} <small>{m['category']} · {m['blp'].get('w','?')}×{m['blp'].get('h','?')} → 2×</small></h3>
 <div class="row {tile}">
  <figure><div class="img" style="background-image:url('orig/{m['id']}.png')"></div><figcaption>2004 (original)</figcaption></figure>
  <figure><div class="img" style="background-image:url('faithful/{m['id']}.png')"></div><figcaption>Faithful</figcaption></figure>
  <figure><div class="img" style="background-image:url('detailed/{m['id']}.png')"></div><figcaption>Detailed</figcaption></figure>
 </div>
</section>""")
    page = f"""<!doctype html><meta charset="utf-8"><title>Valley of Trials — texture lab</title>
<style>
 body{{background:#14161c;color:#ddd;font:14px -apple-system,system-ui,sans-serif;margin:24px}}
 h1{{color:#e4c063}} h3{{font-weight:500;margin:28px 0 8px}} small{{color:#888}}
 .row{{display:flex;gap:14px;flex-wrap:wrap}} figure{{margin:0}} figcaption{{text-align:center;color:#aaa;margin-top:4px}}
 .img{{width:384px;height:384px;background-size:384px 384px;background-repeat:no-repeat;border-radius:6px;image-rendering:auto}}
 .tile .img{{background-size:192px 192px;background-repeat:repeat}}
 nav a{{color:#e4c063;margin-right:14px}}
</style>
<h1>Valley of Trials — your textures, upgraded</h1>
<p>Each texture as the game would show it at the same size: the 2004 original, <b>Faithful</b> (sharper, same
painted look and colours) and <b>Detailed</b> (adds fine grain). Tiling textures are shown 2×2 so you can
check the seams. These files are yours only: never share them.</p>
<nav><a href="#terrain">Ground</a><a href="#buildings">Buildings</a><a href="#plants">Plants</a><a href="#props">Props</a></nav>
{''.join(rows)}
"""
    out = work / "compare.html"
    out.write_text(page)
    print(f"wrote {out} ({len(rows)} textures)")
    if sys.platform == "darwin" and not a.no_open:
        subprocess.run(["open", str(out)])


def main():
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    sub = p.add_subparsers(dest="cmd", required=True)
    z = sub.add_parser("zone")
    z.add_argument("--data", required=True)
    z.add_argument("--extract", required=True, help="the benilla-extract binary")
    z.add_argument("--map", default="Kalimdor")
    z.add_argument("--tiles", required=True, help="X0-X1,Y0-Y1 (ADT tile numbers)")
    z.add_argument("--out", required=True)
    u = sub.add_parser("upscale")
    u.add_argument("--work", required=True)
    u.add_argument("--esrgan", required=True, help="the realesrgan-ncnn-vulkan binary")
    u.add_argument("--models")
    u.add_argument("--limit", type=int)
    u.add_argument("--force", action="store_true")
    c = sub.add_parser("compare")
    c.add_argument("--work", required=True)
    c.add_argument("--no-open", action="store_true")
    a = p.parse_args()
    {"zone": cmd_zone, "upscale": cmd_upscale, "compare": cmd_compare}[a.cmd](a)


if __name__ == "__main__":
    main()
