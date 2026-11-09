"""Signal checks for sparse legacy ids; no game files are required."""
import unittest
import generate_registry_id_adapter as g


class SparseRegistryTests(unittest.TestCase):
    def test_missing_zero_is_not_silently_renumbered(self):
        old = {'minecraft:speed': 1, 'minecraft:slowness': 2}
        current = {'minecraft:speed': 0, 'minecraft:slowness': 1, 'minecraft:new': 2}
        self.assertEqual(g.table(old, current), [g.NONE, 0, 1])
        self.assertEqual(g.table(current, old), [1, 2, g.NONE])
        self.assertEqual(g.registry_names(old), ['', 'minecraft:speed', 'minecraft:slowness'])
        self.assertEqual(g.runs(g.table(old, current)), [[1, 0, 2]])

    def test_alias_reverse_prefers_surviving_original_at_sparse_id(self):
        old = {'removed': 2, 'surviving': 5}
        current = {'surviving': 0}
        self.assertEqual(g.reverse_table(old, current, {'removed': 'surviving'}), [5])
        self.assertEqual(g.table(current, old), [5])

    def test_dense_reports_keep_original_mapping(self):
        self.assertEqual(g.table({'a': 0, 'b': 1}, {'b': 0, 'a': 1}), [1, 0])
        self.assertEqual(g.entries({'x': {'entries': {'a': {'protocol_id': 1}}}}, 'x'), {'a': 1})
        for ids in [(-1, 1), (1, 1), (0, g.NONE)]:
            with self.assertRaises(AssertionError):
                g.entries({'x': {'entries': {'a': {'protocol_id': ids[0]}, 'b': {'protocol_id': ids[1]}}}}, 'x')


if __name__ == '__main__':
    unittest.main()
