# Issues

```
[INFO][snapdown] Found 'downloadMemories('' at file byte index 1212396 (buffer byte index 58)
[INFO][snapdown] File byte index 1212414: Parsing 2 bytes for tag '','... (is_last=true)
[INFO][snapdown] File byte index 1212416: Parsing 16384 bytes for tag '','... (is_last=false)
[INFO][snapdown] Found '',' at file byte index 1212667 (buffer byte index 251)
[ERROR][snapdown] Extracted download link did not start with https: tps://us-east1-aws.api.snapchat.com/dmd/mm?uid=d3658ad1-6f2d-4ca6-a6da-a83bf040ae02&sid=928ADB0A-9BBA-432B-AE98-06DFE2D14B51&mid=928ADB0A-9BBA-432B-AE98-06DFE2D14B51&ts=1765945546005&sig=fe355f477276bd95bdbbb9963a5b5267397d5a077fb830ae77c3c394387745ee
```

The problem here is that although I successfully found the start of a link, it was two bytes before the end of a chunk. Those two bytes contained `ht`, and when we started parsing the next chunk, we somehow didn't include those two bytes.