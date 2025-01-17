### Extracting JSON embedded within strings

```tremor
let example = { "snot": "{\"snot\": \"badger\"" };
match example of
  case extraction=%{ snot ~= json|| } => extraction.snot
  default => "no match"
end;
```

When executed this will result in:

```tremor
"badger"
```

### Decoding base64 embedded within strings

```tremor
let example = { "snot": "eyJzbm90IjogImJhZGdlciJ9Cg==" };
match example of
  case extraction=%{ snot ~= base64|| } => extraction.snot
  default => "no match"
end;
```

When executed this will result in:

```tremor
"{\"snot\": \"badger\"}
```

### Wrap and Extract

We can decode the base64 decoded string through composition:

```tremor
let example = { "snot": "eyJzbm90IjogImJhZGdlciJ9Cg==" };
match example of
  case decoded = %{ snot ~= base64|| } =>
    match { "snot": decoded.snot } of
      case json = %{ snot ~= json|| } => json.snot.snot
      default => "no match - json"
    end
  default => "no match - base64"
end;

```


