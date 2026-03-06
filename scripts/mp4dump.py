#!/usr/bin/env python3
"""Dump MP4 box/atom tree as JSON or flat diff-friendly text.

Usage: mp4dump.py <file.mp4> [--flat] [--no-fields]

Output modes:
  (default)    JSON with full box tree, parsed fields, and data hashes
  --flat       One line per box: path size fields hash (ideal for diff)
  --no-fields  Omit parsed field values, just show box structure and hashes
"""

import hashlib
import json
import struct
import sys

# Boxes that contain other boxes (container/branch nodes)
CONTAINER_TYPES = {
    "moov", "trak", "mdia", "minf", "stbl", "dinf", "edts",
    "mvex", "moof", "traf", "mfra", "skip", "meta", "ipro",
    "sinf", "schi", "rinf", "srpp", "strk", "udta", "ilst",
}

# Full boxes (have version + flags after the header)
FULL_BOX_TYPES = {
    "mvhd", "tkhd", "mdhd", "hdlr", "smhd", "vmhd", "dref",
    "stsd", "stts", "stss", "stsc", "stsz", "stco", "co64",
    "ctts", "elst", "mehd", "trex", "mfhd", "tfhd", "tfdt",
    "trun", "sbgp", "sgpd", "meta",
}


def read_boxes(f, start, end):
    """Read a sequence of boxes from file region [start, end)."""
    boxes = []
    f.seek(start)
    while f.tell() < end:
        box = read_box(f, end)
        if box is None:
            break
        boxes.append(box)
    return boxes


def read_box(f, parent_end):
    """Read a single box at the current file position."""
    offset = f.tell()
    remaining = parent_end - offset
    if remaining < 8:
        return None

    header = f.read(8)
    if len(header) < 8:
        return None

    size, box_type = struct.unpack(">I4s", header)
    box_type = box_type.decode("ascii", errors="replace")
    header_size = 8

    if size == 1:
        # 64-bit extended size
        ext = f.read(8)
        if len(ext) < 8:
            return None
        size = struct.unpack(">Q", ext)[0]
        header_size = 16
    elif size == 0:
        # Box extends to end of file/parent
        size = parent_end - offset

    if size < header_size:
        return None

    data_start = offset + header_size
    data_end = offset + size

    box = {
        "type": box_type,
        "size": size,
        "offset": offset,
    }

    # Check for full box (version + flags)
    is_full = box_type in FULL_BOX_TYPES
    if is_full and (data_end - data_start) >= 4:
        vf = f.read(4)
        version = vf[0]
        flags = int.from_bytes(vf[1:4], "big")
        box["version"] = version
        box["flags"] = flags
        data_start = f.tell()

    if box_type in CONTAINER_TYPES:
        box["children"] = read_boxes(f, data_start, data_end)
    else:
        # Parse known fields or hash the payload
        fields = parse_fields(f, box_type, box.get("version", 0), data_start, data_end)
        if fields:
            box["fields"] = fields

        # Always include a hash of the raw box data for diffing
        payload_size = data_end - data_start
        if payload_size > 0 and payload_size <= 100 * 1024 * 1024:
            f.seek(data_start)
            h = hashlib.md5()
            to_read = payload_size
            while to_read > 0:
                chunk = f.read(min(to_read, 65536))
                if not chunk:
                    break
                h.update(chunk)
                to_read -= len(chunk)
            box["data_hash"] = h.hexdigest()

    f.seek(data_end)
    return box


def parse_fields(f, box_type, version, start, end):
    """Parse fields for well-known box types."""
    f.seek(start)
    size = end - start
    fields = {}

    try:
        if box_type == "ftyp":
            if size >= 8:
                major = f.read(4).decode("ascii", errors="replace")
                minor = struct.unpack(">I", f.read(4))[0]
                fields["major_brand"] = major
                fields["minor_version"] = minor
                brands = []
                while f.tell() + 4 <= end:
                    brands.append(f.read(4).decode("ascii", errors="replace"))
                fields["compatible_brands"] = brands

        elif box_type == "mvhd":
            if version == 1:
                fields["creation_time"] = struct.unpack(">Q", f.read(8))[0]
                fields["modification_time"] = struct.unpack(">Q", f.read(8))[0]
                fields["timescale"] = struct.unpack(">I", f.read(4))[0]
                fields["duration"] = struct.unpack(">Q", f.read(8))[0]
            else:
                fields["creation_time"] = struct.unpack(">I", f.read(4))[0]
                fields["modification_time"] = struct.unpack(">I", f.read(4))[0]
                fields["timescale"] = struct.unpack(">I", f.read(4))[0]
                fields["duration"] = struct.unpack(">I", f.read(4))[0]
            fields["rate"] = struct.unpack(">i", f.read(4))[0] / 65536.0
            fields["volume"] = struct.unpack(">h", f.read(2))[0] / 256.0
            f.read(10)  # reserved
            matrix = struct.unpack(">9i", f.read(36))
            fields["matrix"] = [m / 65536.0 if i < 6 else m / 1073741824.0 for i, m in enumerate(matrix)]
            f.read(24)  # pre_defined
            fields["next_track_id"] = struct.unpack(">I", f.read(4))[0]

        elif box_type == "tkhd":
            if version == 1:
                fields["creation_time"] = struct.unpack(">Q", f.read(8))[0]
                fields["modification_time"] = struct.unpack(">Q", f.read(8))[0]
                fields["track_id"] = struct.unpack(">I", f.read(4))[0]
                f.read(4)  # reserved
                fields["duration"] = struct.unpack(">Q", f.read(8))[0]
            else:
                fields["creation_time"] = struct.unpack(">I", f.read(4))[0]
                fields["modification_time"] = struct.unpack(">I", f.read(4))[0]
                fields["track_id"] = struct.unpack(">I", f.read(4))[0]
                f.read(4)  # reserved
                fields["duration"] = struct.unpack(">I", f.read(4))[0]
            f.read(8)  # reserved
            fields["layer"] = struct.unpack(">h", f.read(2))[0]
            fields["alternate_group"] = struct.unpack(">h", f.read(2))[0]
            fields["volume"] = struct.unpack(">h", f.read(2))[0] / 256.0
            f.read(2)  # reserved
            matrix = struct.unpack(">9i", f.read(36))
            fields["matrix"] = [m / 65536.0 if i < 6 else m / 1073741824.0 for i, m in enumerate(matrix)]
            fields["width"] = struct.unpack(">I", f.read(4))[0] / 65536.0
            fields["height"] = struct.unpack(">I", f.read(4))[0] / 65536.0

        elif box_type == "mdhd":
            if version == 1:
                fields["creation_time"] = struct.unpack(">Q", f.read(8))[0]
                fields["modification_time"] = struct.unpack(">Q", f.read(8))[0]
                fields["timescale"] = struct.unpack(">I", f.read(4))[0]
                fields["duration"] = struct.unpack(">Q", f.read(8))[0]
            else:
                fields["creation_time"] = struct.unpack(">I", f.read(4))[0]
                fields["modification_time"] = struct.unpack(">I", f.read(4))[0]
                fields["timescale"] = struct.unpack(">I", f.read(4))[0]
                fields["duration"] = struct.unpack(">I", f.read(4))[0]
            lang = struct.unpack(">H", f.read(2))[0]
            # ISO 639-2/T language code packed as three 5-bit chars
            c1 = chr(((lang >> 10) & 0x1F) + 0x60)
            c2 = chr(((lang >> 5) & 0x1F) + 0x60)
            c3 = chr((lang & 0x1F) + 0x60)
            fields["language"] = c1 + c2 + c3

        elif box_type == "hdlr":
            f.read(4)  # pre_defined
            handler = f.read(4).decode("ascii", errors="replace")
            fields["handler_type"] = handler
            f.read(12)  # reserved
            remaining = end - f.tell()
            if remaining > 0:
                name = f.read(remaining)
                fields["name"] = name.rstrip(b"\x00").decode("utf-8", errors="replace")

        elif box_type == "stts":
            count = struct.unpack(">I", f.read(4))[0]
            fields["entry_count"] = count
            entries = []
            for _ in range(min(count, 10000)):
                sc, sd = struct.unpack(">II", f.read(8))
                entries.append({"sample_count": sc, "sample_delta": sd})
            fields["entries"] = entries

        elif box_type == "stsc":
            count = struct.unpack(">I", f.read(4))[0]
            fields["entry_count"] = count
            entries = []
            for _ in range(min(count, 10000)):
                fc, spc, sdi = struct.unpack(">III", f.read(12))
                entries.append({"first_chunk": fc, "samples_per_chunk": spc, "sample_description_index": sdi})
            fields["entries"] = entries

        elif box_type == "stsz":
            sample_size = struct.unpack(">I", f.read(4))[0]
            count = struct.unpack(">I", f.read(4))[0]
            fields["sample_size"] = sample_size
            fields["sample_count"] = count
            if sample_size == 0 and count > 0:
                sizes = []
                for _ in range(min(count, 100000)):
                    sizes.append(struct.unpack(">I", f.read(4))[0])
                fields["entry_sizes"] = sizes

        elif box_type in ("stco", "co64"):
            count = struct.unpack(">I", f.read(4))[0]
            fields["entry_count"] = count
            offsets = []
            fmt = ">Q" if box_type == "co64" else ">I"
            sz = 8 if box_type == "co64" else 4
            for _ in range(min(count, 100000)):
                offsets.append(struct.unpack(fmt, f.read(sz))[0])
            fields["chunk_offsets"] = offsets

        elif box_type == "stss":
            count = struct.unpack(">I", f.read(4))[0]
            fields["entry_count"] = count
            entries = []
            for _ in range(min(count, 100000)):
                entries.append(struct.unpack(">I", f.read(4))[0])
            fields["sync_samples"] = entries

        elif box_type == "ctts":
            count = struct.unpack(">I", f.read(4))[0]
            fields["entry_count"] = count
            entries = []
            for _ in range(min(count, 10000)):
                sc, co = struct.unpack(">Ii" if version == 1 else ">II", f.read(8))
                entries.append({"sample_count": sc, "sample_offset": co})
            fields["entries"] = entries

        elif box_type == "elst":
            count = struct.unpack(">I", f.read(4))[0]
            fields["entry_count"] = count
            entries = []
            for _ in range(min(count, 1000)):
                if version == 1:
                    dur, mt = struct.unpack(">Qq", f.read(16))
                else:
                    dur, mt = struct.unpack(">Ii", f.read(8))
                rate = struct.unpack(">h", f.read(2))[0]
                frac = struct.unpack(">h", f.read(2))[0]
                entries.append({
                    "segment_duration": dur,
                    "media_time": mt,
                    "media_rate": rate + frac / 65536.0,
                })
            fields["entries"] = entries

    except (struct.error, UnicodeDecodeError):
        pass  # Partial parse is fine

    return fields if fields else None


def dump_file(path, include_fields):
    with open(path, "rb") as f:
        f.seek(0, 2)
        file_size = f.tell()
        boxes = read_boxes(f, 0, file_size)

    if not include_fields:
        strip_fields(boxes)

    return boxes


def strip_fields(boxes):
    for box in boxes:
        box.pop("fields", None)
        if "children" in box:
            strip_fields(box["children"])


def flat_format_value(val):
    """Format a field value compactly for the flat output."""
    if isinstance(val, list):
        if not val:
            return "[]"
        if isinstance(val[0], dict):
            # List of dicts: one compact repr per entry
            parts = []
            for item in val:
                inner = ",".join(f"{k}={v}" for k, v in item.items())
                parts.append(f"({inner})")
            return "[" + " ".join(parts) + "]"
        # Simple list
        return "[" + ",".join(str(v) for v in val) + "]"
    if isinstance(val, float):
        return f"{val:g}"
    return str(val)


def flat_dump(boxes, path=""):
    """Emit one line per box: path/type size=N hash=H field=val ..."""
    for box in boxes:
        box_path = f"{path}/{box['type']}" if path else box["type"]
        parts = [box_path, f"size={box['size']}"]

        if "version" in box:
            parts.append(f"v={box['version']}")
        if "flags" in box:
            parts.append(f"flags={box['flags']}")

        if box.get("fields"):
            for key, val in box["fields"].items():
                parts.append(f"{key}={flat_format_value(val)}")

        if box.get("data_hash"):
            parts.append(f"hash={box['data_hash']}")

        print(" ".join(parts))

        if "children" in box:
            flat_dump(box["children"], box_path)


def main():
    args = sys.argv[1:]
    flat = "--flat" in args
    no_fields = "--no-fields" in args
    paths = [a for a in args if not a.startswith("--")]

    if not paths:
        print(__doc__, file=sys.stderr)
        sys.exit(1)

    for path in paths:
        boxes = dump_file(path, not no_fields)
        if flat:
            if len(paths) > 1:
                print(f"=== {path} ===")
            flat_dump(boxes)
        else:
            result = {
                "file": path,
                "boxes": boxes,
            }
            indent = 2 if len(paths) == 1 else None
            print(json.dumps(result, indent=indent))


if __name__ == "__main__":
    main()
