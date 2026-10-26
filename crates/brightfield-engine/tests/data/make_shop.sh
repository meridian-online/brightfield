#!/bin/sh
# Regenerates shop.sqlite: tables `customers` (3 rows) and `orders` (2 rows).
set -eu
here=$(cd "$(dirname "$0")" && pwd)
rm -f "$here/shop.sqlite"
/usr/bin/sqlite3 "$here/shop.sqlite" <<'SQL'
CREATE TABLE customers (id INTEGER, name TEXT);
INSERT INTO customers VALUES (1,'Ana'),(2,'Ben'),(3,'Cy');
CREATE TABLE orders (id INTEGER, customer_id INTEGER, total REAL);
INSERT INTO orders VALUES (10,1,9.5),(11,3,20.0);
SQL
