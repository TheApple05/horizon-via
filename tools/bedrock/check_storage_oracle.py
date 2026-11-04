"""Compare Rust storage decoding with hash-pinned MIT Go primitives, offline.

Developer only: requires explicitly supplied existing reference sources. Imports
no world registries, assets or game data, starts no listener and never downloads.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess

SOURCES = {
    'dragonfly-encode.go': '3984f50dfbad7a903be0259aa6aa19918931b87a336e0f6b61db0a7b70224353',
    'dragonfly-encoding.go': '668e43ec5e0be74d3990daa1809ca70372e8157ab902a76484e81e1f7779d3d2',
    'dragonfly-paletted_storage.go': 'a85bc54b35c5cb8bc6be0886e02c2818e5f7fa44ab1657fb35c67dbaef21dbcf',
    'dragonfly-palette.go': 'f83b41d262b6c6604c0e5908e83f669e79b2c648b41838d64a639131f333e27e',
    'gophertunnel-varint.go': 'db56743f9849fa7724b47e90cdbe7fbb16605cbcd435d749448fb65402436e3e',
    'dragonfly-LICENSE.txt': '9b0866098f4b7bfadafa43adec71dae35968053ceaea0487fb4b23c46cc72755',
    'gophertunnel-LICENSE.txt': '5d4f9d510977551f891dfc086b10c9530d9c90cf299e15a9d28f2f7a6c79d85c',
}
DRIVER = r'''
package main
import (
 "bytes"
 "encoding/json"
 "fmt"
 "os"
 "horizon-storage-oracle/protocol"
)
type paletteEncoding interface{}
type Encoding interface { network() byte; encodePalette(*bytes.Buffer, *Palette, paletteEncoding) }
type networkEncoding struct{}
type vector struct { Bits int `json:"bits"`; Bytes []int `json:"bytes"`; Expected []uint32 `json:"expected"` }
func main() {
 cases := []vector{}
 for _, setting := range [][2]int{{0,1},{1,2},{2,3},{3,5},{4,9},{5,17},{6,33},{8,65},{16,257}} {
  width, count := setting[0], setting[1]
  ids := make([]uint32, count)
  for i := range ids { ids[i] = uint32(i)*0x9e3779b9 ^ 0xa5a55a5a }
  p := newPalette(paletteSize(width), ids)
  s := newPalettedStorage(make([]uint32, paletteSize(width).uint32s()), p)
  for x:=byte(0); x<16; x++ { for y:=byte(0); y<16; y++ { for z:=byte(0); z<16; z++ {
   slot := (int(x)*997 + int(y)*11 + int(z)*29) % count
   s.setPaletteIndex(x,y,z,uint16(slot))
  } } }
  raw := bytes.NewBuffer([]byte{9,1,252})
  encodePalettedStorage(raw,s,nil,networkEncoding{},nil)
  v := vector{Bits:width, Expected:make([]uint32,0,4096)}
  for _, b := range raw.Bytes() { v.Bytes=append(v.Bytes,int(b)) }
  for x:=byte(0); x<16; x++ { for z:=byte(0); z<16; z++ { for y:=byte(0); y<16; y++ {
   expected := ids[(int(x)*997 + int(y)*11 + int(z)*29) % count]
   if s.At(x,y,z) != expected { panic(fmt.Sprintf("reference storage mismatch width%d",width)) }
   v.Expected=append(v.Expected,expected)
  } } }
  cases=append(cases,v)
 }
 if err:=json.NewEncoder(os.Stdout).Encode(cases); err!=nil { panic(err) }
}
'''


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--reference', required=True, type=Path)
    parser.add_argument('--out', required=True, type=Path, help='new private output directory')
    parser.add_argument('--target-dir', required=True, type=Path)
    args = parser.parse_args()
    reference = args.reference.resolve(strict=True)
    source = {}
    for name, expected in SOURCES.items():
        path = reference / name
        if path.is_symlink() or not path.is_file() or path.stat().st_size > 256 * 1024:
            raise ValueError('Missing, linked or oversized reference: ' + name)
        data = path.read_bytes()
        if hashlib.sha256(data).hexdigest() != expected:
            raise ValueError('Reference digest differs: ' + name)
        source[name] = data.decode('utf-8')
    output = args.out.resolve()
    output.mkdir(parents=True, exist_ok=False)
    module = output / 'go'; module.mkdir()
    (module / 'go.mod').write_text('module horizon-storage-oracle\n\ngo 1.23.0\n', encoding='utf-8')
    for name in ('dragonfly-palette.go', 'dragonfly-paletted_storage.go'):
        text = source[name]
        if not text.startswith('package chunk\n'):
            raise ValueError('Reference package header differs')
        # Only the package name changes; all actual reference methods are intact.
        text = text.replace('package chunk\n', 'package main\n', 1)
        (module / name).write_text(text, encoding='utf-8', newline='\n')
    (module / 'protocol').mkdir()
    (module / 'protocol' / 'varint.go').write_text(source['gophertunnel-varint.go'], encoding='utf-8', newline='\n')
    for name in ('dragonfly-LICENSE.txt', 'gophertunnel-LICENSE.txt'):
        (module / name).write_text(source[name], encoding='utf-8', newline='\n')
    # Exact pinned reference functions, without importing disk codecs/world registries.
    encode = ''.join(source['dragonfly-encode.go'].splitlines(keepends=True)[96:112])
    palette = ''.join(source['dragonfly-encoding.go'].splitlines(keepends=True)[194:203])
    (module / 'main.go').write_text(DRIVER + '\n' + encode + '\n' + palette, encoding='utf-8', newline='\n')
    env = os.environ.copy()
    env.update(GOMAXPROCS='2', GOFLAGS='-p=2', GOPROXY='off', GOSUMDB='off',
               GOWORK='off', GOTOOLCHAIN='local', GOCACHE=str(output / 'go-cache'),
               GOMODCACHE=str(output / 'go-mod-cache'), CARGO_BUILD_JOBS='2',
               CARGO_TARGET_DIR=str(args.target_dir.resolve()))
    go = shutil.which('go')
    cargo = shutil.which('cargo')
    if not go or not cargo:
        raise ValueError('Existing Go and Cargo installations are required')
    run = subprocess.run([go, 'run', '.'], cwd=module, env=env, capture_output=True, check=True, timeout=180)
    (output / 'go.stderr.log').write_bytes(run.stderr)
    vectors = json.loads(run.stdout)
    if len(vectors) != 9 or sum(len(v['expected']) for v in vectors) != 36864:
        raise ValueError('Reference vector domain differs')
    vector_path = output / 'vectors.json'; vector_path.write_bytes(run.stdout)
    manifest = Path(__file__).resolve().parents[2] / 'bedrock' / 'Cargo.toml'
    checked = subprocess.run([cargo, 'run', '--offline', '--locked', '--manifest-path', str(manifest),
        '-j', '2', '--example', 'check_storage_vectors', '--', str(vector_path)], env=env,
        capture_output=True, check=True, timeout=180)
    (output / 'rust.stdout.log').write_bytes(checked.stdout)
    (output / 'rust.stderr.log').write_bytes(checked.stderr)
    report = {'scope': 'synthetic offline storage conformance; no game/server interoperability claim',
        'reference_sha256': SOURCES, 'vectors_sha256': hashlib.sha256(run.stdout).hexdigest(),
        'widths': [v['bits'] for v in vectors], 'cells': 36864, 'passed': True}
    (output / 'result.json').write_text(json.dumps(report, indent=2) + '\n', encoding='utf-8')
    print(json.dumps(report))


if __name__ == '__main__':
    main()
