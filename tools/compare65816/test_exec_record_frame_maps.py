import copy
import unittest

from exec_record_frame_maps import validate


class FrameMaps(unittest.TestCase):
    fields = {'scratch': {'offset': 0x80, 'size': 64}}
    pointers = (0x80, 0x83, 0x86)

    def routine(self):
        return dict(id=0, fixed_frame=8, spill_bytes=6, arguments=[dict(offset=0, size=3, body_displacement=12)],
                    objects=[dict(displacement=1, size=2)],
                    temporaries=[dict(id=0, size=2, home=dict(kind='direct_page', offset=0xa0)),
                                 dict(id=1, size=3, home=dict(kind='direct_page', offset=0xa2)),
                                 dict(id=2, size=3, home=dict(kind='stack', displacement=3))],
                    calls=[dict(outgoing=5, transfer_peak=3)], local_stack_peak=16,
                    whole_task_stack_bound=None)

    def check(self, rows):
        validate(rows, self.fields, self.pointers)

    def test_mixed_homes_in_calling_routine_have_physical_maps(self):
        self.check([self.routine()])
        # Sharing a home at different live ranges is a compiler proof, not
        # serialized information. The consumer must not invent interference.
        row = self.routine(); row['temporaries'][1]['home']['offset'] = 0xa0
        self.check([row])

    def test_legacy_leaf_pointer_geometry_is_separate(self):
        row = self.routine(); row.update(calls=[], local_stack_peak=8)
        row['temporaries'] = [dict(id=i, size=3, home=dict(kind='direct_page', offset=at))
                              for i, at in enumerate(self.pointers)]
        self.check([row])
        row.update(calls=[dict(outgoing=5, transfer_peak=3)], local_stack_peak=16)
        with self.assertRaises(ValueError): self.check([row])
        row.update(calls=[], local_stack_peak=8)
        row['temporaries'][1]['home']['offset'] = 0xa0
        with self.assertRaises(ValueError): self.check([row])

    def test_direct_page_forgeries_and_old_geometry_are_rejected(self):
        for size, offset in ((1, 0xa0), (4, 0xa0), (2, 0x9e), (2, 0xbf),
                             (3, 0xa3), (3, 0xbe), (3, 0x87), (2, True)):
            row = self.routine(); row['temporaries'][0].update(size=size, home=dict(kind='direct_page', offset=offset))
            with self.subTest(size=size, offset=offset), self.assertRaises(ValueError): self.check([row])
        with self.assertRaises(ValueError): validate([self.routine()], self.fields, (0x80, 0x84, 0x88))

    def test_frame_arguments_peaks_identities_and_unknown_fields_are_checked(self):
        paths = [('fixed_frame', 9), ('spill_bytes', 10), ('local_stack_peak', 15),
                 ('whole_task_stack_bound', 16), ('id', True)]
        for field, value in paths:
            row = self.routine(); row[field] = value
            with self.subTest(field=field), self.assertRaises(ValueError): self.check([row])
        for field, value in (('body_displacement', 11), ('size', 245)):
            row = self.routine(); row['arguments'][0][field] = value
            with self.assertRaises(ValueError): self.check([row])
        for displacement in (0, 7):
            row = self.routine(); row['temporaries'][2]['home']['displacement'] = displacement
            with self.assertRaises(ValueError): self.check([row])
        row = self.routine(); row['temporaries'][1]['id'] = 0
        with self.assertRaises(ValueError): self.check([row])
        row = self.routine(); row['temporaries'][0]['home']['extra'] = 1
        with self.assertRaises(ValueError): self.check([row])
        for outgoing, transfer in ((0, 3), (4, 3), (257, 3), (5, 4)):
            row = self.routine(); row['calls'] = [dict(outgoing=outgoing, transfer_peak=transfer)]
            row['local_stack_peak'] = 8+outgoing+transfer
            with self.assertRaises(ValueError): self.check([row])
        with self.assertRaises(ValueError): self.check([self.routine(), copy.deepcopy(self.routine())])


if __name__ == '__main__':
    unittest.main()
