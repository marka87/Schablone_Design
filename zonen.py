"""Pad- und Paste-Layer überlagern, Dickenzonen einzeichnen und prüfen. Alles lokal.

python zonen.py pads.gbr paste.gbr     (ohne Argumente: Selbsttest)
Maus: Rechteck ziehen = Zone (Toolbar-Zoom/Pan muss aus sein)
Tasten: 1-7 Dicke wählen | u letzte Zone löschen | s Bericht + zonen.json speichern
"""
import json
import sys

import matplotlib.pyplot as plt
from gerbonara import GerberFile
from gerbonara.utils import MM
from matplotlib.patches import Rectangle
from matplotlib.widgets import RectangleSelector

from schablone import DICKEN, max_dicke

plt.rcParams["keymap.save"] = []  # 's' gehört uns, PNG-Export bleibt in der Toolbar


def boxen(pfad):
    res = []
    for o in GerberFile.open(pfad).objects:
        (x0, y0), (x1, y1) = o.bounding_box(unit=MM)
        res.append((x0, y0, x1, y1))
    return res


def mitte_in(a, b):
    """Mittelpunkt von Box b liegt in Box a."""
    x, y = (b[0] + b[2]) / 2, (b[1] + b[3]) / 2
    return a[0] <= x <= a[2] and a[1] <= y <= a[3]


def vergleich(pads, paste):
    # ponytail: O(n²), reicht für einige tausend Pads, danach Raster-Index
    ohne_paste = [p for p in pads if not any(mitte_in(p, b) for b in paste)]
    ohne_pad = [b for b in paste if not any(mitte_in(p, b) for p in pads)]
    return ohne_paste, ohne_pad


def bericht(zonen, paste):
    def limit(ids):
        return min(max_dicke(paste[k][2] - paste[k][0], paste[k][3] - paste[k][1]) for k in ids)

    zeilen, frei, doppelt = [], set(range(len(paste))), 0
    for i, z in enumerate(zonen, 1):
        drin = {k for k, b in enumerate(paste) if mitte_in(z, b)}
        doppelt += len(drin - frei)  # schon in früherer Zone
        frei -= drin
        if not drin:
            zeilen.append(f"Zone {i}: {z[4]:g} mm, keine Aperturen")
            continue
        l = limit(drin)
        zeilen.append(f"Zone {i}: {z[4]:g} mm, {len(drin)} Aperturen, Limit {l:.3f} mm -> "
                      + ("OK" if z[4] <= l else "ZU DICK"))
    if frei:
        l = limit(frei)
        ok = [d for d in DICKEN if d <= l]
        zeilen.append(f"Rest ohne Zone: {len(frei)} Aperturen, Limit {l:.3f} mm, "
                      f"Vorschlag {ok[-1] if ok else 'keine Standarddicke passt'}")
    if doppelt:
        zeilen.append(f"ACHTUNG: {doppelt} Aperturen liegen in mehreren Zonen")
    return "\n".join(zeilen)


def app(pads, paste, ziel="zonen.json"):
    ohne_paste, ohne_pad = vergleich(pads, paste)
    fig, ax = plt.subplots(figsize=(11, 8))
    box = lambda b, **kw: Rectangle(b[:2], b[2] - b[0], b[3] - b[1], **kw)
    for b in pads:
        ax.add_patch(box(b, fc="#7bc47f", ec="none", alpha=.6))
    for b in ohne_paste:
        ax.add_patch(box(b, fc="none", ec="orange", ls="--", lw=1.5))
    for b in paste:
        ax.add_patch(box(b, fc="none", ec="#1f4e9c", lw=1))
    for b in ohne_pad:
        ax.add_patch(box(b, fc="red", ec="red", alpha=.6))
    ax.autoscale()
    ax.set_aspect("equal")
    ax.set_xlabel("grün = Pad | blau = Paste | orange gestrichelt = Pad ohne Paste | rot = Paste ohne Pad  (mm)")

    zonen, artists, aktiv = [], [], [2]

    def titel():
        ax.set_title(f"Dicke {DICKEN[aktiv[0]]:g} mm (Taste 1-{len(DICKEN)}) | Rechteck ziehen = Zone | "
                     "u = zurück | s = speichern")
        fig.canvas.draw_idle()

    def hinzu(x0, y0, x1, y1, i=None):
        i = aktiv[0] if i is None else i
        x0, x1 = sorted((x0, x1))
        y0, y1 = sorted((y0, y1))
        zonen.append((x0, y0, x1, y1, DICKEN[i]))
        c = plt.cm.tab10(i)
        r = Rectangle((x0, y0), x1 - x0, y1 - y0, fc=c, alpha=.25, ec=c, lw=2)
        ax.add_patch(r)
        artists.append((r, ax.text(x0, y1, f"{DICKEN[i]:g} mm", color=c, va="bottom")))
        fig.canvas.draw_idle()

    def maus(e1, e2):
        tb = getattr(fig.canvas, "toolbar", None)
        if (tb is not None and str(tb.mode)) or None in (e1.xdata, e2.xdata):
            return
        hinzu(e1.xdata, e1.ydata, e2.xdata, e2.ydata)

    def taste(e):
        k = e.key or ""
        if k.isdigit() and 1 <= int(k) <= len(DICKEN):
            aktiv[0] = int(k) - 1
            titel()
        elif k == "u" and zonen:
            zonen.pop()
            for a in artists.pop():
                a.remove()
            fig.canvas.draw_idle()
        elif k == "s":
            with open(ziel, "w") as f:
                json.dump([dict(zip(("x0", "y0", "x1", "y1", "dicke"), z)) for z in zonen], f, indent=1)
            print(f"\n{bericht(zonen, paste)}\n(gespeichert: {ziel})")

    fig._sel = RectangleSelector(ax, maus, useblit=True, button=[1], minspanx=.05, minspany=.05,
                                 spancoords="data", interactive=False)
    fig.canvas.mpl_connect("key_press_event", taste)
    titel()
    return fig, hinzu, zonen


if __name__ == "__main__":
    if len(sys.argv) < 3:
        z = bericht([(-1, -1, 1, 1, 0.2), (9, 9, 14, 14, 0.2)], [(0, 0, .5, .5), (10, 10, 13, 13)])
        assert "ZU DICK" in z.splitlines()[0] and z.splitlines()[1].endswith("OK"), z
        print("Selbsttest ok")
    else:
        app(boxen(sys.argv[1]), boxen(sys.argv[2]))
        plt.show()
