"""Content digest of an SQLite database for the installer end-to-end job.

Usage: e2e-sqlite-digest.py <memory.db>

Prints one JSON line: {"rows": {<table>: <count>, ...}, "sha256": <hex>}.
The digest covers the schema and every row (sqlite3 iterdump), so two
stores are equal exactly when their memories are, however much of the
content still sits in the -wal file. The store itself is never opened: the
database and its -wal file are copied into a temporary folder and the copy
is read (SQLite rebuilds the -shm index from the -wal). Run it only while no
program writes to the store (reliable-installers task 8.2).
"""

import hashlib
import json
import os
import shutil
import sqlite3
import sys
import tempfile


def digest(db):
    with tempfile.TemporaryDirectory() as tmp:
        copy = os.path.join(tmp, "store.db")
        shutil.copyfile(db, copy)
        if os.path.exists(db + "-wal"):
            shutil.copyfile(db + "-wal", copy + "-wal")
        con = sqlite3.connect(copy)
        try:
            h = hashlib.sha256()
            for line in con.iterdump():
                h.update(line.encode("utf-8") + b"\n")
            names = [
                r[0]
                for r in con.execute(
                    "SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name"
                )
            ]
            rows = {}
            for name in names:
                quoted = '"' + name.replace('"', '""') + '"'
                rows[name] = con.execute("SELECT count(*) FROM " + quoted).fetchone()[0]
        finally:
            con.close()
    return {"rows": rows, "sha256": h.hexdigest()}


if __name__ == "__main__":
    if len(sys.argv) != 2:
        sys.exit("usage: e2e-sqlite-digest.py <database>")
    print(json.dumps(digest(sys.argv[1]), sort_keys=True))
