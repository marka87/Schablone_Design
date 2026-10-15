import sys
import json
import tkinter as tk
from tkinter import filedialog

def choose_files():
    root = tk.Tk()
    root.withdraw()
    root.attributes('-topmost', True)
    root.lift()
    root.focus_force()
    files = filedialog.askopenfilenames(
        title="SMT / Gerber / ODB++ / ZIP-Dateien auswählen",
        filetypes=[
            ("Alle Schablonen/CAD-Dateien", "*.zip;*.ger;*.gbr;*.pho;*.tgz;*.tar.gz;*.*"),
            ("Gerber-Dateien (*.ger, *.gbr, *.pho)", "*.ger;*.gbr;*.pho"),
            ("ZIP / ODB++ Archive (*.zip, *.tgz)", "*.zip;*.tgz;*.tar.gz"),
            ("Alle Dateien (*.*)", "*.*")
        ]
    )
    root.destroy()
    return list(files)

def choose_folder():
    root = tk.Tk()
    root.withdraw()
    root.attributes('-topmost', True)
    root.lift()
    root.focus_force()
    folder = filedialog.askdirectory(title="Ordner mit Gerber- oder ODB++-Dateien auswählen")
    root.destroy()
    return folder

if __name__ == "__main__":
    mode = sys.argv[1] if len(sys.argv) > 1 else "files"
    if mode == "folder":
        res = choose_folder()
        print(json.dumps(res if res else ""))
    else:
        res = choose_files()
        print(json.dumps(res if res else []))

