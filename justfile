build:
    docker compose build

up:
    docker compose up

down:
    docker compose down

logs:
    docker compose logs -f

rebuild:
    docker compose build --no-cache

restart:
    docker compose down && docker compose up
