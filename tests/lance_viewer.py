import os

from typing import Optional
from pathlib import Path

import lancedb
import pandas as pd

# usage from other file:
# from importlib.machinery import SourceFileLoader
# lance_viewer = SourceFileLoader("lance_viewer", "/Users/$USER/RustroverProjects/refact-lsp/tests/lance_viewer.py").load_module()
# df = lance_viewer.lance2df()


def lance2df(
        database: Optional[str] = None,
        table: Optional[str] = None,
) -> pd.DataFrame:
    cache_dir = Path(f"/Users/{os.getenv('USER')}/.cache/refact/refact_vecdb_cache")
    assert cache_dir.exists(), f"Cache directory {cache_dir} does not exist"

    databases = list(cache_dir.iterdir())
    assert len(databases) > 0, f"No databases found in cache directory"
    if database:
        assert database in databases, f"Database {database} not found in cache directory; found {databases}"
    else:
        database = databases[0]

    uri = cache_dir / database
    assert uri.exists(), f"Database {database} not found in cache directory; found {list(uri.iterdir())}"

    db = lancedb.connect(uri)
    assert len(list(db.table_names())) > 0, f"No tables found in database {database}"

    if table:
        assert table in db.table_names(), f"Table {table} not found in database {database}; found {db.table_names()}"
    else:
        table = list(db.table_names())[0]

    df = db.open_table(table).to_pandas()

    return df


if __name__ == "__main__":
    df = lance2df()
    print(f"df.shape: {df.shape}")
