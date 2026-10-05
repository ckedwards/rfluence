---
rfluence:
  title: rfluence image API test
  url: https://tech-accounts11.atlassian.net/wiki/spaces/rfluencete/pages/131074
  space_key: rfluencete
  updated: 2026-10-04T02:19:37.110Z
  simplified: true
---

# rfluence image API test

rfluence test page: checks images uploaded via the API. Kept as a reference.

## A: green.png, minimal node (no size or layout)

Expected: wide green rectangle

[image: green.png]

## B: green.png with layout=center width=150 widthType=pixel

Expected: small green rectangle, about 150px wide

[image: green.png]

## C: square.svg, fileId of version 2

Expected: blue, VERSION 2

[image: square.svg]

## D: square.svg, fileId of version 1 (old)

Shows red (VERSION 1) or blue (VERSION 2)?

[image: square.svg]

## E: external image

Expected: Wikimedia PNG transparency demo (dice)

![dice](https://upload.wikimedia.org/wikipedia/commons/4/47/PNG_transparency_demonstration_1.png)
