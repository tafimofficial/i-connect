import math
import os
from PIL import Image, ImageChops, ImageDraw, ImageFilter

ASSETS_DIR = os.path.dirname(os.path.abspath(__file__))
ROOT_DIR = os.path.dirname(ASSETS_DIR)
ANDROID_RES = os.path.join(ROOT_DIR, "android_client", "res")

PC_RAW = r"C:\Users\T\.gemini\antigravity\brain\1c04ef7d-9a99-42da-b777-5749cec2b3e5\i_connect_pc_server_icon_1795619024957.jpg"


def make_squircle_mask(size: int, radius: int) -> Image.Image:
    mask = Image.new("L", (size, size), 0)
    draw = ImageDraw.Draw(mask)
    draw.rounded_rectangle((0, 0, size - 1, size - 1), radius=radius, fill=255)
    return mask


def prepare_pc_icon():
    im = Image.open(PC_RAW).convert("RGBA")
    w, h = im.size
    side = min(w, h)
    # Trim 3% margin on each edge so the white outer corners from the JPEG
    # sit completely outside the rounded squircle mask (zero bright corner fringe).
    trim = int(side * 0.03)
    im = im.crop((
        (w - side) // 2 + trim,
        (h - side) // 2 + trim,
        (w + side) // 2 - trim,
        (h + side) // 2 - trim,
    ))
    im = im.resize((1024, 1024), Image.Resampling.LANCZOS)

    mask = make_squircle_mask(1024, radius=212)
    r, g, b, _ = im.split()
    cleaned = Image.merge("RGBA", (r, g, b, mask))

    # Crisp subtle lime inner rim around the squircle
    rim = Image.new("RGBA", (1024, 1024), (0, 0, 0, 0))
    rim_draw = ImageDraw.Draw(rim)
    rim_draw.rounded_rectangle(
        (6, 6, 1017, 1017),
        radius=208,
        outline=(163, 230, 53, 95),
        width=4,
    )
    cleaned = Image.alpha_composite(cleaned, rim)

    pc_png = os.path.join(ASSETS_DIR, "pc_icon.png")
    pc_ico = os.path.join(ASSETS_DIR, "pc_icon.ico")
    cleaned.save(pc_png, "PNG")
    cleaned.save(
        pc_ico,
        format="ICO",
        sizes=[(256, 256), (128, 128), (64, 64), (48, 48), (32, 32), (16, 16)],
    )
    print(f"Saved PC PNG: {pc_png}")
    print(f"Saved PC ICO: {pc_ico}")


def add_glow(base: Image.Image, glow_layer: Image.Image, blur_radius: float, alpha_scale: float = 1.0) -> Image.Image:
    blurred = glow_layer.filter(ImageFilter.GaussianBlur(blur_radius))
    if alpha_scale != 1.0:
        r, g, b, a = blurred.split()
        a = a.point(lambda p: min(255, int(p * alpha_scale)))
        blurred = Image.merge("RGBA", (r, g, b, a))
    return Image.alpha_composite(base, blurred)


def draw_arc_segment(draw: ImageDraw.ImageDraw, cx: float, cy: float, radius: float,
                     start_deg: float, end_deg: float, width: int, color):
    bbox = (cx - radius, cy - radius, cx + radius, cy + radius)
    draw.arc(bbox, start=start_deg, end=end_deg, fill=color, width=width)


def build_phone_icon(size: int = 1024, full_bleed_bg: bool = False) -> Image.Image:
    # Render at 2x supersampling (2048x2048) for razor-sharp anti-aliased vectors
    S = 2048
    LIME = (182, 255, 0, 255)
    LIME_MID = (150, 235, 20, 255)
    LIME_SOFT = (182, 255, 0, 170)

    canvas = Image.new("RGBA", (S, S), (0, 0, 0, 255))
    draw = ImageDraw.Draw(canvas)

    # Subtle radial AMOLED green ambience in center
    ambience = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    amb_draw = ImageDraw.Draw(ambience)
    amb_draw.ellipse((420, 320, S - 420, S - 320), fill=(140, 255, 0, 36))
    ambience = ambience.filter(ImageFilter.GaussianBlur(190))
    canvas = Image.alpha_composite(canvas, ambience)

    # Smartphone silhouette centered in canvas (generous ~16% safe margin for adaptive icons)
    px0, py0, px1, py1 = 560, 250, S - 560, S - 250
    phone_r = 128

    # Outer lime bloom behind phone body
    phone_glow = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    pg_draw = ImageDraw.Draw(phone_glow)
    pg_draw.rounded_rectangle((px0, py0, px1, py1), radius=phone_r, outline=(165, 255, 0, 230), width=32)
    canvas = add_glow(canvas, phone_glow, blur_radius=55, alpha_scale=0.95)
    canvas = add_glow(canvas, phone_glow, blur_radius=20, alpha_scale=0.8)

    # Carbon metallic phone chassis
    chassis = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    ch_draw = ImageDraw.Draw(chassis)
    ch_draw.rounded_rectangle((px0, py0, px1, py1), radius=phone_r, fill=(18, 22, 16, 255))

    # Carbon-fiber diagonal weave texture inside chassis
    weave = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    w_draw = ImageDraw.Draw(weave)
    for k in range(-S, S * 2, 24):
        w_draw.line((k, 0, k - S, S), fill=(42, 54, 34, 65), width=7)
        w_draw.line((k, 0, k + S, S), fill=(10, 14, 9, 80), width=7)
    chassis_mask = Image.new("L", (S, S), 0)
    ImageDraw.Draw(chassis_mask).rounded_rectangle((px0, py0, px1, py1), radius=phone_r, fill=255)
    weave.putalpha(ImageChops.multiply(weave.split()[3], chassis_mask))
    chassis = Image.alpha_composite(chassis, weave)

    # Inner AMOLED glass screen
    sx0, sy0, sx1, sy1 = px0 + 48, py0 + 52, px1 - 48, py1 - 52
    scr_r = 88
    ch_draw = ImageDraw.Draw(chassis)
    ch_draw.rounded_rectangle((sx0, sy0, sx1, sy1), radius=scr_r, fill=(5, 8, 4, 255), outline=(140, 230, 30, 210), width=10)

    # Subtle glossy diagonal sheen across upper-left screen
    sheen = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    sh_draw = ImageDraw.Draw(sheen)
    sh_draw.polygon([(sx0 - 40, sy0 - 40), (sx1 + 40, sy0 - 40), (sx0 - 40, sy0 + 780)], fill=(210, 255, 170, 20))
    scr_mask = Image.new("L", (S, S), 0)
    ImageDraw.Draw(scr_mask).rounded_rectangle((sx0, sy0, sx1, sy1), radius=scr_r, fill=255)
    sheen.putalpha(ImageChops.multiply(sheen.split()[3], scr_mask))
    chassis = Image.alpha_composite(chassis, sheen)

    # Crisp neon lime outer phone bezel outline
    ch_draw = ImageDraw.Draw(chassis)
    ch_draw.rounded_rectangle((px0, py0, px1, py1), radius=phone_r, outline=LIME, width=16)

    # Side power/volume accent ticks in lime
    ch_draw.rounded_rectangle((px1 - 4, py0 + 340, px1 + 14, py0 + 490), radius=6, fill=LIME_MID)
    ch_draw.rounded_rectangle((px1 - 4, py0 + 535, px1 + 14, py0 + 640), radius=6, fill=LIME_MID)

    # Dynamic pill speaker / camera notch & home indicator bar
    cx = S // 2
    cy = S // 2 + 12
    ch_draw.rounded_rectangle((cx - 105, sy0 + 38, cx + 105, sy0 + 68), radius=15, fill=(22, 32, 18, 255), outline=(130, 210, 40, 170), width=4)
    ch_draw.ellipse((cx + 58, sy0 + 44, cx + 76, sy0 + 62), fill=LIME)
    ch_draw.rounded_rectangle((cx - 115, sy1 - 44, cx + 115, sy1 - 28), radius=8, fill=(163, 230, 53, 190))

    canvas = Image.alpha_composite(canvas, chassis)

    # Center Emblem Layer (glow + crisp vector) matching the PC Icon's "I" + Signal Waves + Link Nodes!
    emblem = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    ed = ImageDraw.Draw(emblem)

    # 1. Symmetrical Wireless Signal Waves (left & right of the central "I")
    for r_wave, w_wave in [(260, 26), (345, 26)]:
        draw_arc_segment(ed, cx - 24, cy, r_wave, 138, 222, w_wave, LIME)
        draw_arc_segment(ed, cx + 24, cy, r_wave, -42, 42, w_wave, LIME)

    # 2. Diagonal Link Nodes (4 corners connecting into the "I" core)
    node_offsets = [(-285, -285), (285, -285), (-285, 285), (285, 285)]
    for ox, oy in node_offsets:
        nx, ny = cx + ox, cy + oy
        tx, ty = cx + int(ox * 0.44), cy + int(oy * 0.44)
        ed.line((nx, ny, tx, ty), fill=LIME, width=24)
        ed.ellipse((nx - 42, ny - 42, nx + 42, ny + 42), fill=(6, 10, 4, 255), outline=LIME, width=20)

    # 3. Bold Futuristic Geometric "I" Monogram with Chamfered Serifs & Glassy Inner Core
    ed.rounded_rectangle((cx - 156, cy - 340, cx + 156, cy - 240), radius=16, fill=(140, 235, 30, 95), outline=LIME, width=18)
    ed.rounded_rectangle((cx - 156, cy + 240, cx + 156, cy + 340), radius=16, fill=(140, 235, 30, 95), outline=LIME, width=18)
    ed.rounded_rectangle((cx - 74, cy - 255, cx + 74, cy + 255), radius=16, fill=(22, 34, 14, 245), outline=LIME, width=18)
    ed.rounded_rectangle((cx - 28, cy - 196, cx + 28, cy + 196), radius=12, fill=(182, 255, 0, 225))
    ed.line((cx - 9, cy - 182, cx - 9, cy + 182), fill=(238, 255, 190, 230), width=10)

    # Diagonal tech slash highlights across top and bottom serifs
    ed.line((cx - 58, cy - 235, cx + 110, cy - 348), fill=(5, 8, 4, 255), width=26)
    ed.line((cx - 110, cy + 348, cx + 58, cy + 235), fill=(5, 8, 4, 255), width=26)
    ed.line((cx - 58, cy - 235, cx + 110, cy - 348), fill=LIME, width=9)
    ed.line((cx - 110, cy + 348, cx + 58, cy + 235), fill=LIME, width=9)

    # Composite glow + sharp emblem
    canvas = add_glow(canvas, emblem, blur_radius=34, alpha_scale=0.9)
    canvas = add_glow(canvas, emblem, blur_radius=12, alpha_scale=0.7)
    canvas = Image.alpha_composite(canvas, emblem)

    out = canvas.resize((size, size), Image.Resampling.LANCZOS)
    if not full_bleed_bg:
        mask = make_squircle_mask(size, radius=int(size * 0.205))
        r, g, b, _ = out.split()
        out = Image.merge("RGBA", (r, g, b, mask))
        rim = Image.new("RGBA", (size, size), (0, 0, 0, 0))
        ImageDraw.Draw(rim).rounded_rectangle(
            (int(size * 0.008), int(size * 0.008), size - int(size * 0.008) - 1, size - int(size * 0.008) - 1),
            radius=int(size * 0.201),
            outline=(163, 230, 53, 85),
            width=max(2, size // 256),
        )
        out = Image.alpha_composite(out, rim)
    return out


def prepare_phone_icons():
    phone_squircle = build_phone_icon(1024, full_bleed_bg=False)
    phone_full = build_phone_icon(1024, full_bleed_bg=True)

    phone_png = os.path.join(ASSETS_DIR, "phone_icon.png")
    phone_squircle.save(phone_png, "PNG")
    print(f"Saved Phone PNG: {phone_png}")

    # Standard Android mipmaps (squircle + circular round icon)
    sizes = {
        "mipmap-mdpi": 48,
        "mipmap-hdpi": 72,
        "mipmap-xhdpi": 96,
        "mipmap-xxhdpi": 144,
        "mipmap-xxxhdpi": 192,
    }
    for folder, sz in sizes.items():
        out_dir = os.path.join(ANDROID_RES, folder)
        os.makedirs(out_dir, exist_ok=True)

        sq = phone_squircle.resize((sz, sz), Image.Resampling.LANCZOS)
        sq.save(os.path.join(out_dir, "ic_launcher.png"), "PNG")

        # True circular crop with lime border for ic_launcher_round
        rnd_base = phone_full.resize((sz, sz), Image.Resampling.LANCZOS)
        circ_mask = Image.new("L", (sz, sz), 0)
        ImageDraw.Draw(circ_mask).ellipse((0, 0, sz - 1, sz - 1), fill=255)
        r, g, b, _ = rnd_base.split()
        rnd = Image.merge("RGBA", (r, g, b, circ_mask))
        rnd.save(os.path.join(out_dir, "ic_launcher_round.png"), "PNG")
        print(f"Saved Android {folder} ({sz}x{sz})")


if __name__ == "__main__":
    prepare_pc_icon()
    prepare_phone_icons()
