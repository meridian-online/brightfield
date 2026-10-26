#!/bin/sh
# Regenerates book.xlsx, the workbook the sources tests read. It holds two sheets:
#   "Empty"  - a sheet with no cells at all
#   "Orders" - row 1 blank, row 2 the header (id, item, qty), rows 3-5 three data rows
# Built from hand-written minimal OOXML parts, so no spreadsheet library is needed.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
tmp=$(mktemp -d)
mkdir -p "$tmp/_rels" "$tmp/xl/_rels" "$tmp/xl/worksheets"
cat > "$tmp/[Content_Types].xml" <<'X'
<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/><Override PartName="/xl/worksheets/sheet2.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/></Types>
X
cat > "$tmp/_rels/.rels" <<'X'
<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>
X
cat > "$tmp/xl/workbook.xml" <<'X'
<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Empty" sheetId="1" r:id="rId1"/><sheet name="Orders" sheetId="2" r:id="rId2"/></sheets></workbook>
X
cat > "$tmp/xl/_rels/workbook.xml.rels" <<'X'
<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet2.xml"/></Relationships>
X
cat > "$tmp/xl/worksheets/sheet1.xml" <<'X'
<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData/></worksheet>
X
s() { printf '<c r="%s" t="inlineStr"><is><t>%s</t></is></c>' "$1" "$2"; }
n() { printf '<c r="%s"><v>%s</v></c>' "$1" "$2"; }
{
  printf '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>'
  printf '<row r="2">%s%s%s</row>' "$(s A2 id)" "$(s B2 item)" "$(s C2 qty)"
  printf '<row r="3">%s%s%s</row>' "$(n A3 1)" "$(s B3 apple)" "$(n C3 5)"
  printf '<row r="4">%s%s%s</row>' "$(n A4 2)" "$(s B4 pear)" "$(n C4 7)"
  printf '<row r="5">%s%s%s</row>' "$(n A5 3)" "$(s B5 plum)" "$(n C5 9)"
  printf '</sheetData></worksheet>'
} > "$tmp/xl/worksheets/sheet2.xml"
rm -f "$here/own_files_book.tmp" "$here/book.xlsx"
(cd "$tmp" && /usr/bin/zip -X -q -r "$here/book.xlsx" '[Content_Types].xml' _rels xl)
rm -rf "$tmp"
