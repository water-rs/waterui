"""Android CSV frame statistics captured on Pixel 9 Pro (API 36)."""

import unittest

from metrics.frames import parse_framestats, parse_render_nodes
from metrics.frame_profile import COUNTERS, STAGES, parse_frames, parse_stages, summarize


CAPTURE = '---PROFILEDATA---\nFlags,FrameTimelineVsyncId,IntendedVsync,Vsync,InputEventId,HandleInputStart,AnimationStart,PerformTraversalsStart,DrawStart,FrameDeadline,FrameStartTime,FrameInterval,WorkloadTarget,SyncQueued,SyncStart,IssueDrawCommandsStart,SwapBuffers,FrameCompleted,DequeueBufferDuration,QueueBufferDuration,GpuCompleted,SwapBuffersCompleted,DisplayPresentTime,CommandSubmissionCompleted,\n0,1797877,129048706565956,129048714895569,-915818465,129048716077781,129048716182924,129048732026999,129048732117535,129048739828200,129048716075461,8329613,16600000,129048732148378,129048732196514,129048732594952,129048733202374,129048736215150,6958,120809,129048736215150,129048733359519,129048750359804,129048733202374,\n0,1797899,129048723248363,129048731578988,-800566217,129048732698508,129048732814190,129048748577455,129048748657452,129048748157813,129048732696107,8330625,16600000,129048748681378,129048748734560,129048748957908,129048749495627,129048752439719,4720,102946,129048752439719,129048749631980,129048766976055,129048749495627,\n---PROFILEDATA---'


class FrameStatisticsTests(unittest.TestCase):
    def test_device_csv_uses_completion_and_reported_deadlines(self):
        result = parse_framestats(CAPTURE)
        self.assertEqual(result["frame_samples"], 2)
        self.assertEqual(result["missed_slots"], 1)
        self.assertEqual(result["frame_ms_p50"], 29.649)

    def test_nonordinary_frames_do_not_enter_the_distribution(self):
        sample = CAPTURE.replace("\n0,", "\n1,")
        self.assertEqual(parse_framestats(sample), {})

    def test_view_count_is_not_a_retained_node_count(self):
        result = parse_render_nodes("8 views, 13.80 kB of render nodes")
        self.assertEqual(result, {"android_view_count": 8, "display_list_kb": 13.8})

    def test_retained_nodes_are_a_gauge_not_accumulated_work(self):
        counters = " ".join(f"{name}=1" for name in COUNTERS)
        lines = "\n".join(
            f"hydrolysis::frame_profile: hwui frame {counters} live_render_nodes={n}"
            for n in (8, 10, 8)
        )
        result = summarize(parse_frames(lines))
        self.assertEqual(result["live_render_nodes_max"], 10)
        self.assertEqual(result["live_render_nodes_p50"], 8)
        self.assertNotIn("live_render_nodes_total", result)
        self.assertEqual(result["jni_calls_total"], 3)

    def test_stage_timings_keep_fractional_milliseconds_separate_from_counts(self):
        line = "hydrolysis::frame_profile: hwui stages " + " ".join(
            f"{name}=0.125" for name in STAGES
        )
        self.assertEqual(parse_frames(line), [])
        result = summarize(parse_stages(line), STAGES, "frame_stage_samples")
        self.assertEqual(result["frame_stage_samples"], 1)
        self.assertEqual(result["layout_ms_p99"], 0.125)
        self.assertNotIn("layout_ms_total", result)

    def test_incomplete_stage_record_fails(self):
        with self.assertRaisesRegex(ValueError, "publication_ms"):
            parse_stages("hydrolysis::frame_profile: hwui stages total_ms=1.25")

    def test_missing_timing_columns_fail_explicitly(self):
        with self.assertRaisesRegex(ValueError, "FrameInterval"):
            parse_framestats(CAPTURE.replace("FrameInterval", "Unknown"))


if __name__ == "__main__":
    unittest.main()
