@echo off
cd /d C:\dev\verio
cargo clippy --workspace -- -D warnings > C:\dev\verio\clippy7.log 2>&1
cargo test --workspace > C:\dev\verio\test7.log 2>&1
echo ALLDONE > C:\dev\verio\verify_done.flag