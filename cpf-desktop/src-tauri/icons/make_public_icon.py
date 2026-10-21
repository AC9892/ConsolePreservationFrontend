"""Draw a generic dual-screen device icon for the public CPF DSi build."""
import struct
from pathlib import Path

size = 64
pixels = [[(0, 0, 0, 0) for _ in range(size)] for _ in range(size)]

def rect(left, top, right, bottom, color):
    for y in range(top, bottom):
        for x in range(left, right):
            pixels[y][x] = color

outline = (161, 168, 174, 255)
body = (69, 73, 78, 255)
screen = (20, 22, 24, 255)
rect(6, 4, 58, 29, outline)
rect(8, 6, 56, 27, body)
rect(17, 9, 47, 25, screen)
rect(9, 30, 55, 34, outline)
rect(6, 35, 58, 60, outline)
rect(8, 37, 56, 58, body)
rect(17, 39, 47, 55, screen)
rect(10, 44, 15, 51, outline)
rect(9, 46, 16, 49, outline)
for x, y in [(51, 43), (54, 47), (48, 47), (51, 51)]:
    rect(x, y, x + 2, y + 2, outline)

bitmap = bytearray()
for row in reversed(pixels):
    for red, green, blue, alpha in row:
        bitmap.extend((blue, green, red, alpha))
mask = b"\0" * (((size + 31) // 32) * 4 * size)
header = struct.pack("<IiiHHIIiiII", 40, size, size * 2, 1, 32, 0, len(bitmap) + len(mask), 0, 0, 0, 0)
payload = header + bitmap + mask
icon = struct.pack("<HHH", 0, 1, 1) + struct.pack("<BBBBHHII", size, size, 0, 0, 1, 32, len(payload), 22) + payload
Path(__file__).with_name("public-dsi.ico").write_bytes(icon)
