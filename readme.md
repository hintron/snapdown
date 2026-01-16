# SnapDown

Quickly download your SnapChat files.


# Overview

SnapChat provides a way to download all your pictures. However, this download
process is painfully slow. Each file you download requires you to click some
buttons, and it downloads one at a time. This makes it virtually impossible to
download thousands of pictures that you've taken over the years.

What SnapDown does is extract all the download links from the
`memories_history.html` page into a .csv file, and then run a simple program
that will download those links in parallel.

SnapDown is cross platform (it should work on the major operating systems -
Windows, MacOS, and Linux).


# How to use

1. On SnapChat, select the files you want to download, download the thing they
   want you to download, and unzip it.

2. There should be an `index.html` web page. Double click it, so that it opens
   in your web browser (Chrome, Safari, Edge, etc.).

3. On the left, click the `Memories` tab. The address bar should show that you
   opened `memories_history.html`.

4. Do NOT click the "Download All" or any other download links.

5. Open the browser's "console".

   * In Chrome, right click the webpage you just opened and click `Inspect`. Or,
     do Ctrl-Shift-C.

5. Open [`extract_download_links.js`](javascript/extract_download_links.js),
   click "Raw", and copy all the code to your clipboard.

   * You can do this quickly with Ctrl-a, then Ctrl-c.

6. Paste it into the console you just opened.

   * In Chrome, you will need to type `allow pasting` and enter before it will
     allow you to paste code into the console.

7. Save the resulting .csv file somewhere.

8. Run the program with that .csv file as input.

   * It can take some time to download all the files, depending on your internet
     connection speed.

> [!TIP]
> If possible, make sure your computer has an ethernet connection or good WiFi
> connection.
> For a 100 Mbps internet connection (a decent WiFi speed) it can take one hour
> to download 40 GB, and a 200 Mbps internet connection will download that in 30
> minutes.

That's it! You files should now be downloading into a folder.

Make sure that the files look ok. If any files look corrupted, move them into a
separate "backup" folder and rerun SnapDown. SnapDown will only download files
that are missing and skip files that already exist in the output folder.

Remember, the download links being used will apparently expire in 3 days!
