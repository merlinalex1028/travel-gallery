"""Identify the numbered fixture in a bitmap of the isolated viewer window."""
import struct


def check_window_pixels(raw, width, height, background, digits):
    if len(raw) < 54 or raw[:2] != b'BM':
        raise ValueError('Expected a BMP of the isolated window')
    offset, header_size = struct.unpack_from('<II', raw, 10)
    cols, signed_rows, planes, bits, compression = struct.unpack_from('<iiHHI', raw, 18)
    rows = abs(signed_rows)
    stride = ((cols * bits + 31) // 32) * 4
    if (not 1 <= cols <= 8192 or not 1 <= rows <= 8192 or bits not in (24, 32)
            or planes != 1 or compression or header_size < 40
            or not 14 + header_size <= offset <= len(raw) - rows * stride):
        raise ValueError('Unsupported or truncated window bitmap')

    def matches(x, y, expected):
        row = rows - 1 - y if signed_rows > 0 else y
        pos = offset + row * stride + x * (bits // 8)
        return all(abs(actual - target) <= 16 for actual, target in zip(raw[pos:pos + 3], reversed(expected)))

    # The centered, unzoomed photo has broad solid bands above and below its digits.
    # A similarly colored filmstrip thumbnail is too small to qualify.
    center = cols // 2
    left = right = 0
    sample_y = 0
    for y in range(rows // 5, rows * 4 // 5, 4):
        if not matches(center, y, background):
            continue
        start, end = center, center + 1
        while start >= 8 and matches(start - 8, y, background):
            start -= 8
        while start > 0 and matches(start - 1, y, background):
            start -= 1
        while end + 7 < cols and matches(end + 7, y, background):
            end += 8
        while end < cols and matches(end, y, background):
            end += 1
        if end - start > right - left:
            left, right, sample_y = start, end, y
    photo_width = right - left
    if photo_width < max(240, cols // 3):
        raise ValueError('No full size fixture image with the expected background')

    x = left + photo_width // 16
    top, bottom = sample_y, sample_y + 1
    while top > 0 and matches(x, top - 1, background):
        top -= 1
    while bottom < rows and matches(x, bottom, background):
        bottom += 1
    photo_height = bottom - top
    if abs(photo_height - photo_width * height / width) > 4:
        raise ValueError('Fixture is clipped or its aspect ratio differs')

    for digit, pattern in enumerate(digits):
        for cell, filled in enumerate(pattern):
            x = left + (415 + digit * 300 + cell % 3 * 70) * photo_width // width
            y = top + (330 + cell // 3 * 70) * photo_height // height
            if not matches(x, y, (255, 255, 255) if filled == '1' else background):
                raise ValueError(f'Window pixels differ at digit {digit + 1}, cell {cell + 1}')
    return dict(windowPixels=[cols, rows], photoPixels=[photo_width, photo_height], digitCells=45)
