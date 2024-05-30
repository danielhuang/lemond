#!/bin/sh
set -ev

sudo systemctl disable --now lemond
cargo build
sudo -E target/debug/lemond
