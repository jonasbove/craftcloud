"""Rebuild the CraftCloud logo PSD: the Mackerel layer clipped to the Rectangle (a clipping mask),
the Rectangle filled with the brand green, and a fresh flattened preview.

    python3 -I logo_psd.py <in.psd> <out.psd> <rrggbb> <composite.rgba>

The source must have two layers, bottom "Rectangle 1" and top "Mackerel" (the logo's PSD). The
Mackerel's pixels and every layer's extra data (names, effects, metadata) are copied unchanged;
only the Mackerel's clipping flag is set and the Rectangle's colour channels are replaced.
`composite.rgba` is the flattened result as raw 8-bit RGBA, rows top to bottom, canvas-sized
(made with ImageMagick by the caller); it becomes the PSD's merged image.
"""
import struct
import sys


def u16(d, p):
    return struct.unpack(">H", d[p:p + 2])[0]


def u32(d, p):
    return struct.unpack(">I", d[p:p + 4])[0]


def main(src, dst, hexcolor, composite_path):
    d = open(src, "rb").read()
    assert d[:4] == b"8BPS" and u16(d, 4) == 1, "not a PSD (version 1)"
    channels, height, width, depth, mode = struct.unpack(">HIIHH", d[12:26])
    assert depth == 8 and mode == 3, "expected 8-bit RGB"
    header = d[:26]
    p = 26
    cm_end = p + 4 + u32(d, p)
    colour_mode = d[p:cm_end]
    p = cm_end
    ir_end = p + 4 + u32(d, p)
    resources = d[p:ir_end]
    p = ir_end

    lm_len = u32(d, p)
    lm_start = p + 4
    lm_end = lm_start + lm_len
    li_len = u32(d, lm_start)
    li_start = lm_start + 4
    count = struct.unpack(">h", d[li_start:li_start + 2])[0]
    n = abs(count)
    q = li_start + 2
    records = []
    for _ in range(n):
        top, left, bottom, right = struct.unpack(">iiii", d[q:q + 16])
        q += 16
        nch = u16(d, q)
        q += 2
        chans = []
        for _c in range(nch):
            cid = struct.unpack(">h", d[q:q + 2])[0]
            clen = u32(d, q + 2)
            chans.append([cid, clen])
            q += 6
        sig_blend = d[q:q + 8]
        opacity, clipping, flags, filler = d[q + 8], d[q + 9], d[q + 10], d[q + 11]
        q += 12
        extra_len = u32(d, q)
        extra = d[q + 4:q + 4 + extra_len]
        q += 4 + extra_len
        records.append(dict(rect=(top, left, bottom, right), chans=chans, sig_blend=sig_blend, opacity=opacity, clipping=clipping, flags=flags, filler=filler, extra=extra))
    # Channel image data, layer by layer, channel by channel.
    for r in records:
        r["data"] = []
        for cid, clen in r["chans"]:
            r["data"].append((cid, d[q:q + clen]))
            q += clen
    li_end = li_start + li_len
    rest_of_lm = d[li_end:lm_end]  # global layer mask info + tagged blocks, copied as is

    names = [r["extra"] for r in records]
    assert len(records) == 2, f"expected 2 layers, found {len(records)}"
    rect, fish = records
    assert b"Mackerel" in fish["extra"] or b"M\x00a\x00c" in fish["extra"], "top layer is not the Mackerel"

    # Rectangle: same alpha (channel -1), RGB replaced by the brand colour (raw, compression 0).
    rgb = bytes.fromhex(hexcolor)
    top, left, bottom, right = rect["rect"]
    w, h = right - left, bottom - top
    new = []
    for cid, blob in rect["data"]:
        if cid in (0, 1, 2):
            new.append((cid, struct.pack(">H", 0) + bytes([rgb[cid]]) * (w * h)))
        else:
            new.append((cid, blob))
    rect["data"] = new
    # Mackerel: clipped to the layer below.
    fish["clipping"] = 1

    def record_bytes(r):
        out = struct.pack(">iiii", *r["rect"]) + struct.pack(">H", len(r["data"]))
        for cid, blob in r["data"]:
            out += struct.pack(">hI", cid, len(blob))
        out += r["sig_blend"] + bytes([r["opacity"], r["clipping"], r["flags"], r["filler"]])
        out += struct.pack(">I", len(r["extra"])) + r["extra"]
        return out

    li = struct.pack(">h", count) + b"".join(record_bytes(r) for r in records) + b"".join(blob for r in records for _, blob in r["data"])
    if len(li) % 2:
        li += b"\0"
    lm = struct.pack(">I", len(li)) + li + rest_of_lm
    layer_section = struct.pack(">I", len(lm)) + lm

    comp = open(composite_path, "rb").read()
    assert len(comp) == width * height * 4, "composite size doesn't match the canvas"
    planes = b"".join(comp[c::4] for c in range(4))
    merged = struct.pack(">H", 0) + planes
    # The merged image has as many channels as the header says (4: RGB + alpha).
    header = header[:12] + struct.pack(">H", 4) + header[14:]
    open(dst, "wb").write(header + colour_mode + resources + layer_section + merged)
    print(f"wrote {dst}: {len(records)} layers, Mackerel clipped to Rectangle, colour #{hexcolor}")
    _ = names


if __name__ == "__main__":
    main(*sys.argv[1:5])
