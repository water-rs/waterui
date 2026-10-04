tell application "System Events"
    click at {{{from_x}, {from_y}}}
    delay {duration_sec}
    click at {{{to_x}, {to_y}}}
end tell

