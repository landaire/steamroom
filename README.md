# steamroom

Utilities for interacting with Steam's API

## History

This project is a **cleanroom** reimplementation of [DepotDownloader](https://github.com/steamre/depotdownloader).

I originally used an LLM to translate DepotDownloader to Rust, and put all of that [DepotDownloader-rs](https://github.com/landaire/depotdownloader-rs). However, I realized that GPL licensing is a pain in the ass for Rust projects because of static linking, and decided to do the following:

1. Generate docs for the conversion library
2. Delete the source code from the docs
3. Copy that + the file tree and old `ddl` binary to a new repo
4. Instruct a new LLM session how to reverse engineer steam (using Binary Ninja MCP + Steam libs loaded)
5. Told it to reimplement it to the API spec
6. ???
7. 4 Hours later, we're GPL-free

Any major improvements done to this library should, in spirit of collaboration, be shared back to the SteamRE/DepotDownloader project in the spirit of advancing everyone's capabilities.

Not to air my personal grievances with GPL in this README, but DepotDownloader has been a godsend for many projects and I do believe in the spirit of upstreaming changes you make to libraries you use. I don't like the idea of GPL infecting things which statically link against the library, however. And that is the only reason why this library exists as a cleanroom reimplemntation.
