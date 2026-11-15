"""Android CSV frame statistics captured on Pixel 9 Pro (API 36)."""

import unittest

from metrics.frames import parse_framestats


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

    def test_missing_timing_columns_fail_explicitly(self):
        with self.assertRaisesRegex(ValueError, "FrameInterval"):
            parse_framestats(CAPTURE.replace("FrameInterval", "Unknown"))


if __name__ == "__main__":
    unittest.main()
