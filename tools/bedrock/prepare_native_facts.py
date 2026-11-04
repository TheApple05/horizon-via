"""Extract bounded, authenticated CC0 identity/light facts; no assets or mixed catalogs.

Developer tool only. The source directory is an explicit existing native snapshot.
This program never fetches sources, signs in, starts a game or reads player data.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import re

VERSION = '1.26.60'
PROTOCOL = 2225
NATIVE_REVISION = '1e041bf547b7c12e94d2b998f68d5ed146975742'
PROTOCOLGEN_REVISION = '9eed056998235e4c01e0f3befabed651021dea77'
INPUTS = {
    'canonical_block_states.nbt': '88b8c903ac75d46247bed5ff785c0c050068d28440190f7412d697b045d2d51f',
    'block_state_data.json': '1e4e7ca182e0181d7cefabf5abe8736de552ef1a9b623416b4ce24a8bff0f167',
    'item_properties_table.json': '00cf9752282b6607e605ad365974c928df9797c72062de2aeca8f82e658e623e',
    'biome_id_map.json': 'a4d67dd4bd577e0a3f30d785bab061911883c5c4443b0ea4768bd5f7f2f50919',
    'LICENSE': 'a2010f343487d3f7618affe54f789f5487602331c0a8d03f49e9a7c547cf0499',
}
IDENTIFIER = re.compile(r'[a-z0-9_.-]+:[a-z0-9_./-]+\Z')
PROPERTY = re.compile(r'[a-z0-9_:./-]+\Z')


def require(condition, message):
    if not condition:
        raise ValueError(message)


def digest(data):
    return hashlib.sha256(data).hexdigest()


def unique_json(data):
    def pairs(rows):
        out = {}
        for key, value in rows:
            require(key not in out, 'Duplicate JSON field')
            out[key] = value
        return out
    return json.loads(data, object_pairs_hook=pairs)


class Reader:
    """Strict native-palette subset of NetworkLittleEndian NBT, not a game codec."""

    def __init__(self, data: bytes):
        require(len(data) <= 16 * 1024 * 1024, 'Oversized native palette')
        self.data, self.at = data, 0

    def take(self, size):
        require(0 <= size <= len(self.data) - self.at, 'Truncated native palette')
        out = self.data[self.at:self.at + size]
        self.at += size
        return out

    def byte(self):
        return self.take(1)[0]

    def uint(self):
        value = 0
        for index in range(5):
            byte = self.byte()
            require(index != 4 or byte < 16, 'VarInt32 overflow')
            value |= (byte & 127) << (index * 7)
            if byte < 128:
                return value
        raise ValueError('VarInt32 exceeds five bytes')

    def integer(self):
        value = self.uint()
        return (value >> 1) ^ -(value & 1)

    def text(self):
        length = self.uint()
        require(length <= 256, 'Oversized native palette string')
        return self.take(length).decode('utf-8', errors='strict')

    def properties(self):
        out = {}
        while (kind := self.byte()) != 0:
            require(len(out) < 64, 'Too many native state properties')
            key = self.text()
            require(PROPERTY.fullmatch(key) is not None and key not in out,
                    'Invalid or duplicate native state property')
            if kind == 1:
                raw = self.byte()
                value = ['b', raw if raw < 128 else raw - 256]
            elif kind == 3:
                value = ['i', self.integer()]
            elif kind == 8:
                value = ['s', self.text()]
            else:
                raise ValueError('Unsupported native state property type')
            out[key] = value
        return out

    def state(self):
        require(self.byte() == 10 and self.text() == '', 'Expected unnamed root compound')
        out = {}
        while (kind := self.byte()) != 0:
            key = self.text()
            require(key not in out, 'Duplicate native root field')
            if key == 'name' and kind == 8:
                out[key] = self.text()
            elif key == 'states' and kind == 10:
                out[key] = self.properties()
            elif key == 'version' and kind == 3:
                out[key] = self.integer()
            else:
                raise ValueError('Unexpected native root field/type')
        require(set(out) == {'name', 'states', 'version'}, 'Missing native root field')
        require(IDENTIFIER.fullmatch(out['name']) is not None and out['version'] > 0,
                'Invalid native state identity/version')
        return out


def extract(source: Path):
    source = source.resolve(strict=True)
    inputs = {}
    for name, expected in INPUTS.items():
        path = source / name
        require(path.is_file() and not path.is_symlink() and path.stat().st_size <= 16 * 1024 * 1024,
                'Missing, linked or oversized native input')
        data = path.read_bytes()
        require(digest(data) == expected, 'Pinned native input hash differs: ' + name)
        inputs[name] = data
    require(unique_json((source / 'version.json').read_bytes()) ==
            {'version': VERSION, 'protocol': PROTOCOL}, 'Native source version differs')
    measurements = unique_json(inputs['block_state_data.json'])
    raw_items = unique_json(inputs['item_properties_table.json'])
    raw_biomes = unique_json(inputs['biome_id_map.json'])
    require(len(measurements) == 23960 and len(raw_items) == 2080 and len(raw_biomes) == 89,
            'Pinned native catalog domain differs')
    require(set(measurements) == {str(i) for i in range(23960)}, 'Native ordinals are not contiguous')
    reader = Reader(inputs['canonical_block_states.nbt'])
    states, names, hashes, identities = [], set(), set(), set()
    while reader.at < len(reader.data):
        ordinal = len(states)
        require(ordinal < 23960, 'Too many native palette states')
        row = reader.state(); measured = measurements[str(ordinal)]
        runtime_hash = measured['hashedNetworkId']
        require(type(runtime_hash) is int and 0 <= runtime_hash <= 0xffffffff and runtime_hash not in hashes,
                'Invalid or duplicate native hashed identity')
        light = [measured['lightEmission'], measured['lightDamping']]
        require(all(type(v) is int and 0 <= v <= 15 for v in light), 'Invalid native light facts')
        identity = (row['name'], json.dumps(row['states'], sort_keys=True, separators=(',', ':')))
        require(identity not in identities, 'Duplicate native state descriptor')
        identities.add(identity); names.add(row['name']); hashes.add(runtime_hash)
        states.append({'ordinal': ordinal, 'hash': runtime_hash, 'name': row['name'],
                       'properties': row['states'], 'state_version': row['version'], 'light': light})
    require(len(states) == 23960 and len(names) == 1480, 'Native palette coverage differs')
    items, item_ids = [], set()
    for name, raw in sorted(raw_items.items()):
        item_id, stack = raw['id'], raw['maxStackSize']
        require(IDENTIFIER.fullmatch(name) is not None and type(item_id) is int and
                -(1 << 31) <= item_id < (1 << 31) and item_id not in item_ids, 'Invalid native item identity')
        require(type(stack) is int and 1 <= stack <= 64, 'Invalid raw native stack fact')
        item_ids.add(item_id)
        # These are raw measurements, deliberately not asserted as effective stack rules.
        items.append({'name': name, 'id': item_id, 'raw_max_stack': stack})
    require(len(set(raw_biomes.values())) == 89 and all(IDENTIFIER.fullmatch(k) is not None and
            type(v) is int and 0 <= v < (1 << 31) for k, v in raw_biomes.items()), 'Invalid native biome domain')
    result = {'schema': 1, 'version': VERSION, 'protocol': PROTOCOL, 'license': 'CC0-1.0',
              'native_revision': NATIVE_REVISION, 'states': states, 'items': items,
              'biomes': dict(sorted(raw_biomes.items()))}
    return result, inputs['LICENSE']


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source', required=True, type=Path)
    parser.add_argument('--out', required=True, type=Path, help='new output directory; existing output refused')
    args = parser.parse_args()
    result, license_text = extract(args.source)
    payload = (json.dumps(result, ensure_ascii=True, sort_keys=True, separators=(',', ':')) + '\n').encode()
    args.out.mkdir(parents=True, exist_ok=False)
    (args.out / 'native-facts.json').write_bytes(payload)
    (args.out / 'LICENSE-CC0.txt').write_bytes(license_text)
    provenance = {'scope': 'CC0 native identity/state/light facts only; no assets or mixed-input catalog',
        'source': 'https://github.com/didntpot/bedrock-data/tree/' + NATIVE_REVISION + '/release/' + VERSION,
        'protocolgen_revision': PROTOCOLGEN_REVISION, 'input_sha256': INPUTS, 'output_sha256': digest(payload),
        'states': len(result['states']), 'block_names': len({r['name'] for r in result['states']}),
        'items': len(result['items']), 'biomes': len(result['biomes']), 'raw_stack_counts_are_not_effective_rules': True}
    (args.out / 'provenance.json').write_text(json.dumps(provenance, indent=2) + '\n', encoding='utf-8')
    print(json.dumps(provenance))


if __name__ == '__main__':
    main()
