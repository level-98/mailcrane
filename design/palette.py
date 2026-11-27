"""The brand palette, read from colors.json, the single source (#495).

design/build.py draws with these colours and gen_theme.py builds the app
theme from the same file, so neither keeps a copy that can drift. Edit the
colours in colors.json only.
"""

import json
from pathlib import Path

COLORS_PATH = Path(__file__).resolve().parent / "colors.json"

# build.py's names for the dark colours it draws with.
DARK_NAMES = {"darkSurface": "surface", "darkSurfaceHigh": "surfaceHigh", "darkPrimary": "primary"}


def load(path=COLORS_PATH):
    """The light colours by their colors.json names, plus the dark ones in DARK_NAMES."""
    data = json.loads(Path(path).read_text(encoding="utf-8"))
    palette = dict(data["light"])
    for name, dark in DARK_NAMES.items():
        palette[name] = data["dark"][dark]
    return palette
