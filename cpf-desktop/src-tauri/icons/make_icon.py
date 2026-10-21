"""Generate the small, neutral CPF Windows application icon."""

import struct
from pathlib import Path

size = 64
pixels = [[(46, 49, 53, 255) for _ in range(size)] for _ in range(size)]
letters = {
    "C": ["11111", "10000", "10000", "10000", "10000", "10000", "11111"],
    "P": ["11110", "10001", "10001", "11110", "10000", "10000", "10000"],
    "F": ["11111", "10000", "10000", "11110", "10000", "10000", "10000"],
}
for x in range(5, 59):
    for y in (5, 6, 57, 58): pixels[y][x] = (120, 136, 151, 255)
for y in range(5, 59):
    for x in (5, 6, 57, 58): pixels[y][x] = (120, 136, 151, 255)
for index, glyph in enumerate(letters.values()):
    left = 10 + index * 16
    for row, line in enumerate(glyph):
        for column, mark in enumerate(line):
            if mark == "1":
                for dy in range(3):
                    for dx in range(2):
                        pixels[20 + row * 3 + dy][left + column * 2 + dx] = (220, 224, 227, 255)
bitmap = bytearray()
for row in reversed(pixels):
    for r, g, b, a in row:
        bitmap.extend((b, g, r, a))
mask = b"\0" * (((size + 31) // 32) * 4 * size)
header = struct.pack("<IiiHHIIiiII", 40, size, size * 2, 1, 32, 0, len(bitmap) + len(mask), 0, 0, 0, 0)
payload = header + bitmap + mask
icon = struct.pack("<HHH", 0, 1, 1) + struct.pack("<BBBBHHII", size, size, 0, 0, 1, 32, len(payload), 22) + payload
Path(__file__).with_name("icon.ico").write_bytes(icon)
