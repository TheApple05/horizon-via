"""Hand-encoded NetworkLE NBT boundaries; no game, accounts, source fetch or sockets."""
import unittest
from prepare_native_facts import Reader

# Explicit independent wire bytes: unnamed Compound, String name, Compound states,
# ZigZag Int version1, End. Lengths and type tags are protocol facts.
NAME = b'\x08\x04name\x0fminecraft:stone'
STATES = b'\x0a\x06states\x00'
VERSION = b'\x03\x07version\x02'
MINIMAL = b'\x0a\x00' + NAME + STATES + VERSION + b'\x00'
EXPECTED = {'name':'minecraft:stone','states':{},'version':1}


class NativePaletteReaderTests(unittest.TestCase):
    def test_independent_minimal_reordered_and_concatenated_roots(self):
        reordered = b'\x0a\x00' + VERSION + STATES + NAME + b'\x00'
        reader = Reader(MINIMAL + reordered)
        self.assertEqual(reader.state(), EXPECTED)
        self.assertEqual(reader.at, len(MINIMAL))
        self.assertEqual(reader.state(), EXPECTED)
        self.assertEqual(reader.at, len(MINIMAL + reordered))

    def test_signed_byte_zigzag_integer_and_string_keep_distinct_types(self):
        # Synthetic properties exercise the codec; these are not stone gameplay facts.
        properties = b'\x0a\x06states\x01\x04flag\xff\x03\x03age\x0d\x08\x04axis\x01x\x00'
        actual = Reader(b'\x0a\x00' + NAME + properties + VERSION + b'\x00').state()
        self.assertEqual(actual['states'], {'flag':['b',-1],'age':['i',-7],'axis':['s','x']})

    def test_every_truncated_root_is_rejected(self):
        for end in range(len(MINIMAL)):
            with self.subTest(end=end), self.assertRaises(ValueError):
                Reader(MINIMAL[:end]).state()

    def test_varint_boundaries_and_overflow(self):
        self.assertEqual(Reader(b'\x80\x01').uint(),128)
        self.assertEqual(Reader(b'\xff\xff\xff\xff\x0f').uint(),0xffffffff)
        self.assertEqual(Reader(b'\xff\xff\xff\xff\x0f').integer(),-2147483648)
        for invalid in (b'\x80', b'\xff\xff\xff\xff\x10', b'\x80'*6):
            with self.subTest(value=invalid), self.assertRaises(ValueError):
                Reader(invalid).uint()

    def test_invalid_utf8_and_declared_oversized_string_are_rejected(self):
        with self.assertRaises(UnicodeDecodeError):Reader(b'\x01\xff').text()
        with self.assertRaisesRegex(ValueError,'Oversized'):Reader(b'\x81\x02').text()

    def test_root_identity_fields_and_version_are_not_guessed(self):
        malformed = [b'\x09\x00'+NAME+STATES+VERSION+b'\x00',
            b'\x0a\x01x'+NAME+STATES+VERSION+b'\x00',
            b'\x0a\x00'+NAME+STATES+b'\x00',
            b'\x0a\x00'+NAME+STATES+b'\x03\x07version\x00\x00',
            b'\x0a\x00'+NAME+STATES+b'\x03\x07version\x01\x00',
            b'\x0a\x00'+NAME+NAME+STATES+VERSION+b'\x00']
        for value in malformed:
            with self.subTest(value=value), self.assertRaises(ValueError):Reader(value).state()

    def test_duplicate_or_unknown_property_types_are_rejected(self):
        for fields in [b'\x08\x04axis\x01x\x08\x04axis\x01z', b'\x0a\x04nest\x00']:
            value=b'\x0a\x00'+NAME+b'\x0a\x06states'+fields+b'\x00'+VERSION+b'\x00'
            with self.subTest(fields=fields), self.assertRaises(ValueError):Reader(value).state()

    def test_property_count_boundary_accepts64_and_rejects65(self):
        def properties(count):
            data=bytearray(b'\x0a\x06states')
            for i in range(count):
                key=('p'+str(i)).encode('ascii')
                data.extend(b'\x01'+bytes([len(key)])+key+b'\x00')
            return bytes(data)+b'\x00'
        self.assertEqual(len(Reader(b'\x0a\x00'+NAME+properties(64)+VERSION+b'\x00').state()['states']),64)
        with self.assertRaisesRegex(ValueError,'Too many'):
            Reader(b'\x0a\x00'+NAME+properties(65)+VERSION+b'\x00').state()


if __name__=='__main__':
    unittest.main()
