import unittest

from scripts.update_roadmap import build_snapshot


class BuildSnapshotTests(unittest.TestCase):
    def test_counts_and_sorts_milestone_tasks(self):
        issues = [
            {
                "number": 12,
                "title": "[M0-02] Second task",
                "state": "open",
                "html_url": "https://github.com/level-98/mailcrane/issues/12",
            },
            {
                "number": 11,
                "title": "[M0-01] First task",
                "state": "closed",
                "html_url": "https://github.com/level-98/mailcrane/issues/11",
            },
            {
                "number": 40,
                "title": "[P-01] Launch check",
                "state": "open",
                "html_url": "https://github.com/level-98/mailcrane/issues/40",
            },
            {
                "number": 99,
                "title": "Untracked issue",
                "state": "open",
                "html_url": "https://github.com/level-98/mailcrane/issues/99",
            },
        ]

        snapshot = build_snapshot(issues)

        self.assertEqual((snapshot["completed"], snapshot["total"], snapshot["percent"]), (1, 3, 33))
        self.assertEqual([group["id"] for group in snapshot["groups"]], ["M0", "P"])
        self.assertEqual(
            [task["number"] for task in snapshot["groups"][0]["tasks"]],
            [1, 2],
        )

    def test_empty_issue_list_has_zero_progress(self):
        snapshot = build_snapshot([])

        self.assertEqual(snapshot["groups"], [])
        self.assertEqual((snapshot["completed"], snapshot["total"], snapshot["percent"]), (0, 0, 0))

    def test_percentage_rounds_half_up(self):
        issues = [
            {
                "number": number,
                "title": f"[M0-{number:02}] Task {number}",
                "state": "closed" if number == 1 else "open",
                "html_url": f"https://github.com/level-98/mailcrane/issues/{number}",
            }
            for number in range(1, 9)
        ]

        snapshot = build_snapshot(issues)

        self.assertEqual(snapshot["percent"], 13)


if __name__ == "__main__":
    unittest.main()
