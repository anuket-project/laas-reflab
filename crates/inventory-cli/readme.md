docker run --rm \                               
  --network laas-dev_liblaas \
  -v ./inventory:/inventory \
  -e DATABASE_URL="postgres://postgres:password@liblaas-db:5432/liblaas" \
  inventory-cli:latest validate -p /inventory
