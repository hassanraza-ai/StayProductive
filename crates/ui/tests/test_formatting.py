import unittest

from stayproductive.formatting import duration_text


class DurationTextTest(unittest.TestCase):
    def test_short_durations(self):
        self.assertEqual(duration_text(0), "less than a minute")
        self.assertEqual(duration_text(59), "less than a minute")
        self.assertEqual(duration_text(60), "1 min")
        self.assertEqual(duration_text(3599), "59 min")

    def test_hours(self):
        self.assertEqual(duration_text(3600), "1 h")
        self.assertEqual(duration_text(3660), "1 h 01 min")
        self.assertEqual(duration_text(7 * 3600 + 5 * 60), "7 h 05 min")

    def test_negative_is_clamped(self):
        self.assertEqual(duration_text(-30), "less than a minute")


if __name__ == "__main__":
    unittest.main()
