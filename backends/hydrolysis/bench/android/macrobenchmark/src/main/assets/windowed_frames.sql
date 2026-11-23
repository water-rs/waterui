SELECT
    p.name AS process_name,
    a.ts AS actual_start_ns,
    a.ts + a.dur AS actual_end_ns,
    e.ts AS expected_start_ns,
    a.display_frame_token,
    a.present_type,
    a.jank_type,
    sf.ts + sf.dur AS present_ns
FROM actual_frame_timeline_slice a
JOIN process p USING (upid)
LEFT JOIN expected_frame_timeline_slice e
    ON e.upid = a.upid
    AND e.surface_frame_token = a.surface_frame_token
    AND e.layer_name = a.layer_name
LEFT JOIN actual_frame_timeline_slice sf
    ON sf.display_frame_token = a.display_frame_token
    AND sf.layer_name IS NULL
    AND (sf.surface_frame_token IS NULL OR sf.surface_frame_token = 0)
    AND sf.dur > 0
WHERE a.layer_name IS NOT NULL AND a.dur > 0
