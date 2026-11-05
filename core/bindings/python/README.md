# octopus-core

The native core of the [OCTOPUS Live Engine](https://github.com/martinkatamba/octopus_live_engine): the Rust engine that runs live workflow blueprints, built for Python with PyO3.

You don't normally use this package directly. Install the SDK, and it runs on the core whenever `octopus-core` is installed:

```sh
pip install "octopus-live-engine[core]"
```

One wheel per platform serves Python 3.10 and later (abi3). The SDK itself needs Python 3.11 or later.

The core includes the in-memory, Redis, Postgres, MySQL, SQL Server and SQLite stores, and Temporal for async steps. DuckDB is not in the published wheels.

## Licence

MIT; see `LICENSE`. The wheel links Rust crates under their own permissive licences (MIT, Apache-2.0, BSD, ISC and others); their notices are in `THIRD_PARTY_LICENSES.txt`, installed with the package's metadata.
