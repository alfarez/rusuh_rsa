#!/bin/sh
set -e

if [ ! -f .env ]; then
  echo "Error: .env tidak ditemukan"
  exit 1
fi

export $(grep -v '^#' .env | grep -v '^$' | xargs)

docker compose down && docker compose build --no-cache && docker compose up
