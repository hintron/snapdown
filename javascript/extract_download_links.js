// NOTE: This script is intended to be run in the browser console on the "Memories" page (memories_history.html) provided by SnapChat.
// Simply do Shift + Alt + C, copy and paste this file into the console, and double check that things are parsing correctly.


(() => {
  // Select all table rows except the header
  const rows = Array.from(document.querySelector("table").querySelectorAll("tr")).slice(1);

  const out = [];
  out.push([
    "timestamp_utc",
    "format",
    "latitude",
    "longitude",
    "download_url"
  ].join(","));

  let mode = "prompt"; // "prompt" or "auto"
  let processedCount = 0;
  let failureCount = 0;
  let totalRows = rows.length;
  let row_index = -1;

  for (const row of rows) {
    row_index++;
    const cells = row.querySelectorAll("td");
    if (cells.length !== 4) {
      console.error(`Unexpected number of cells in row ${row_index}: ${cells.length} (expected 4)`);
      failureCount++;
      continue;
    }

    const timestamp = cells[0].textContent.trim();
    const format = cells[1].textContent.trim();

    const locMatch = cells[2].textContent.match(
      /([-0-9.]+),\s*([-0-9.]+)/
    );
    if (!locMatch) {
      console.error(`Failed to parse latitude and longitude from row ${row_index}: ${cells[2].textContent.trim()}`);
      failureCount++;
      continue;
    }

    const lat = locMatch[1];
    const lon = locMatch[2];

    const onclick = cells[3]
      .querySelector("a")
      ?.getAttribute("onclick");

    if (!onclick) {
      console.error(`Failed to find onclick attribute in row ${row_index}: ${cells[3].innerHTML}`);
      failureCount++;
      continue;
    }


    const urlMatch = onclick.match(/'(https:\/\/[^']+)'/);
    if (!urlMatch) {
      console.error(`Failed to parse download URL from onclick attribute in row ${row_index}: ${onclick}`);
      failureCount++;
      continue;
    }

    const url = urlMatch[1];
    const rowData = [timestamp, format, lat, lon, url];

    if (mode === "prompt") {
      const message = `Parsed row ${row_index}/${totalRows}:\n\n` +
        `Timestamp: ${timestamp}\n` +
        `Format: ${format}\n` +
        `Latitude: ${lat}\n` +
        `Longitude: ${lon}\n` +
        `Download URL: ${url}\n\n` +
        `Enter your choice:\n` +
        `  * Parse next row (Enter)\n` +
        `  * Cancel (1)\n` +
        `  * Parse remaining rows (2)`;

      const choice = prompt(message);

      if (choice === "1") {
        console.log("Operation cancelled by user.");
        return;
      } else if (choice === "2") {
        mode = "auto";
        console.log(`Parsing remaining ${totalRows - processedCount} rows...`);
      }
      // continue to next row with prompt
    }

    out.push(rowData.map(v => `"${v.replace(/"/g, '""')}"`).join(","));
    processedCount++;

    // Log progress every 5% when in auto mode
    if (mode === "auto") {
      const progress = (processedCount / totalRows) * 100;
      const prevProgress = ((processedCount - 1) / totalRows) * 100;

      // Check if we crossed a 5% threshold
      if (Math.floor(progress / 5) > Math.floor(prevProgress / 5)) {
        console.log(`Progress: ${Math.floor(progress / 5) * 5}% (${processedCount}/${totalRows} rows)`);
      }
    }
  }

  console.log(`Parsing complete. Total rows processed: ${processedCount}/${totalRows}`);
  console.log(`Successes: ${processedCount - failureCount}`);
  console.log(`Failures: ${failureCount}`);


  const blob = new Blob([out.join("\n")], { type: "text/csv" });
  const a = document.createElement("a");
  a.href = URL.createObjectURL(blob);
  a.download = "snap_export.csv";
  a.click();
})();
