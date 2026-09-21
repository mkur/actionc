import unittest
from check_accumulator_forwarding import check_listing

class AccumulatorStreams(unittest.TestCase):
    def setUp(self):
        self.before=[(0x10000,b'\xa3\x02'),(0x10002,b'\x83\x04'),(0x10004,b'\xa3\x04'),
                     (0x10006,b'\x18'),(0x10007,b'\x69\x01\x00'),(0x1000a,b'\x83\x06'),
                     (0x1000c,b'\x5c\x00\x00\x01')]
        self.after=[(0x10000,b'\xa3\x02'),(0x10002,b'\x83\x04'),(0x10004,b'\x18'),
                    (0x10005,b'\x69\x01\x00'),(0x10008,b'\x83\x06'),(0x1000a,b'\x5c\x00\x00\x01')]
    def test_only_declared_loads_and_relocated_targets_may_change(self):
        self.assertEqual(check_listing(self.before,self.after,[0x10004]),{0x10004:0x10004})
        for sites in [[],[0x10005],[0x10004,0x10004],[0x10000]]:
            with self.subTest(sites=sites),self.assertRaises(AssertionError):check_listing(self.before,self.after,sites)
        for i in range(len(self.after)):
            changed=self.after.copy();pc,code=changed[i];changed[i]=(pc,code[:-1]+bytes([code[-1]^1]))
            with self.subTest(i=i),self.assertRaises(AssertionError):check_listing(self.before,changed,[0x10004])
    def test_retained_store_and_no_independent_entry_are_required(self):
        for at,code in [(1,b'\x83\x05'),(2,b'\xa3\x05'),(6,b'\x5c\x04\x00\x01')]:
            old=self.before.copy();old[at]=(old[at][0],code)
            with self.subTest(at=at),self.assertRaises(AssertionError):check_listing(old,self.after,[0x10004])
        old=self.before.copy();old[-1]=(old[-1][0],b'\x5c\x06\x00\x01')
        new=self.after.copy();new[-1]=(new[-1][0],b'\x5c\x04\x00\x01')
        self.assertEqual(check_listing(old,new,[0x10004]),{0x10004:0x10004})

if __name__=='__main__':unittest.main()
