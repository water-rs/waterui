"""Regression checks for the Android benchmark acceptance comparator."""

import unittest

from compare import compare_metric


class PackageSizeGateTests(unittest.TestCase):
    def test_zero_tolerance_allows_improvement_but_never_growth(self):
        for metric in ("apk_bytes", "total_bytes"):
            for candidate, expected in (
                ([90, 95], "pass"),
                ([100, 100], "pass"),
                ([101, 101], "fail"),
                ([90, 101], "fail"),
            ):
                with self.subTest(metric=metric, candidate=candidate):
                    result = compare_metric(metric, [100, 100], candidate, {})
                    self.assertEqual(result["verdict"], expected)


if __name__ == "__main__":
    unittest.main()
