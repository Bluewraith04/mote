<!-- args: -->
<!-- expect: names an inserted slice that is not declared above it -->
## Work package 0: A

- [x] **S1** One. Text.
  needs: none | proof: `true`
- [x] **S1.1** First. Text.
  needs: S1.2 | proof: `true`
- [x] **S1.2** Second. Text.
  needs: S1 | proof: `true`
