[windows]
db:
    docker run -p 6379:6379 --ulimit memlock=-1 docker.dragonflydb.io/dragonflydb/dragonfly

[linux]
db:
    docker run --network=host --ulimit memlock=-1 docker.dragonflydb.io/dragonflydb/dragonfly


