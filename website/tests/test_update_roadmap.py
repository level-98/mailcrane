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

    def test_ios_tasks_are_not_on_the_roadmap(self):
        titles = {
            1: "[M7-02] iOS check on every PR",
            2: "[M6-06] TestFlight beta and App Store release",
            3: "[M1-11] Message list screen, iOS",
            4: "[M7-01] Decide how iOS gets built",
            5: "[M0-16] TestFlight internal builds from CI",
            6: "[M7-03] Message view screen, iOS",
            7: "[M1-10] Message list screen, Android",
        }
        issues = [
            {
                "number": number,
                "title": title,
                "state": "open",
                "html_url": f"https://github.com/level-98/mailcrane/issues/{number}",
            }
            for number, title in titles.items()
        ]

        snapshot = build_snapshot(issues)

        # Decision 0005: M7 and the iOS tasks with older IDs are left out.
        self.assertEqual([group["id"] for group in snapshot["groups"]], ["M1"])
        self.assertEqual([task["id"] for task in snapshot["groups"][0]["tasks"]], ["M1-10"])
        self.assertEqual(snapshot["total"], 1)

    def test_empty_issue_list_has_zero_progress(self):
        snapshot = build_snapshot([])

        self.assertEqual(snapshot["groups"], [])
        self.assertEqual((snapshot["completed"], snapshot["total"], snapshot["percent"]), (0, 0, 0))

    def test_bugs_never_appear_even_with_a_task_id(self):
        issues = [
            {
                "number": 50,
                "title": "[M4-24] A feature",
                "state": "open",
                "labels": [{"name": "m4"}],
                "html_url": "https://github.com/level-98/mailcrane/issues/50",
            },
            {
                "number": 51,
                "title": "[M4-26] Something found along the way",
                "state": "open",
                "labels": [{"name": "bug"}, {"name": "m4"}],
                "html_url": "https://github.com/level-98/mailcrane/issues/51",
            },
        ]

        snapshot = build_snapshot(issues)

        tasks = snapshot["groups"][0]["tasks"]
        self.assertEqual([task["issue_number"] for task in tasks], [50])
        self.assertEqual(snapshot["total"], 1)

    def test_after_1_0_milestone_is_listed(self):
        issues = [
            {
                "number": 61,
                "title": "[M8-01] Attachments browser",
                "state": "open",
                "html_url": "https://github.com/level-98/mailcrane/issues/61",
            },
        ]

        snapshot = build_snapshot(issues)

        self.assertEqual([group["id"] for group in snapshot["groups"]], ["M8"])
        self.assertEqual(snapshot["groups"][0]["title"], "After 1.0")

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
