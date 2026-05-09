"""Generate a large JSON array file for viewer testing.

Usage:
    python scripts/gen_big_json.py [target_mb] [out_path]

Defaults: 300 MB at tests/fixtures/big_300mb.json
Streams records to disk; constant memory.
"""
import json
import os
import sys
import random
import string

target_mb = int(sys.argv[1]) if len(sys.argv) > 1 else 300
out_path = sys.argv[2] if len(sys.argv) > 2 else "tests/fixtures/big_300mb.json"
target_bytes = target_mb * 1024 * 1024

os.makedirs(os.path.dirname(out_path) or ".", exist_ok=True)

rng = random.Random(42)
words = ["alpha", "bravo", "charlie", "delta", "echo", "foxtrot", "golf",
         "hotel", "india", "juliet", "kilo", "lima", "mike", "november",
         "oscar", "papa", "quebec", "romeo", "sierra", "tango"]

def make_record(i: int) -> dict:
    return {
        "id": i,
        "uuid": "".join(rng.choices(string.hexdigits.lower(), k=32)),
        "name": f"{rng.choice(words)}-{rng.choice(words)}-{i}",
        "score": rng.random() * 1000,
        "active": rng.random() > 0.5,
        "tags": rng.sample(words, k=rng.randint(2, 6)),
        "nested": {
            "city": rng.choice(["Beijing", "Shanghai", "Shenzhen", "Hangzhou", "Chengdu"]),
            "lat": rng.uniform(-90, 90),
            "lon": rng.uniform(-180, 180),
            "history": [rng.randint(0, 10000) for _ in range(rng.randint(3, 10))],
        },
        "note": "lorem ipsum " * rng.randint(2, 8),
    }

written = 0
i = 0
with open(out_path, "w", encoding="utf-8", newline="\n") as f:
    f.write("[\n")
    while True:
        rec = json.dumps(make_record(i), ensure_ascii=False)
        prefix = "  " if i == 0 else ",\n  "
        chunk = prefix + rec
        f.write(chunk)
        written += len(chunk.encode("utf-8"))
        i += 1
        if written >= target_bytes:
            break
        if i % 50000 == 0:
            mb = written / 1024 / 1024
            print(f"  {i:>9} records, {mb:7.1f} MB", flush=True)
    f.write("\n]\n")

size = os.path.getsize(out_path)
print(f"done: {out_path}  records={i}  size={size/1024/1024:.1f} MB")
