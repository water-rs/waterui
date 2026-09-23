# Incident Report

## Summary

The application previously crashed with `SIGBUS` during runtime validation.
After linker-path fixes and deterministic stream replay, startup is stable.

## Timeline

| Time (UTC+8) | Event |
| --- | --- |
| 00:58 | `water run` hit undefined `VideoToolbox` symbols |
| 01:13 | Xcode project now injects `-framework VideoToolbox` |
| 01:19 | App boot succeeded with repository CLI |

## Reproduction Script

```bash
water run --platform macos
```

## Notes

> Keep runtime simulation deterministic: fixed chars/s and full-source markdown.

Final verdict: stream parsing path is now testable without pre-sliced chunks.
