# Use Ripscope in scripts

A one-shot search prints one row per match and exits, so a script or CI job can
ask where a function is defined, who calls it or whether anything still uses it.

## Build the index

Build the index once at the start of a job. It prints nothing when it succeeds.

```console
$ ripscope -Rbk
```

`-R` takes the C files in subdirectories too, and `-k` leaves out the system
headers. [Build an index](building.md) covers file lists and include directories.

## Run a search

Add `-d` to answer from the index as it is. Without it, each search first checks
the sources for changes, which a script that just built the index does not need.

```console
$ ripscope -dL -3 greet
main.c main 5 greet("world");
```

A row is the file, the enclosing function, the line number and the source line.
[Queries](queries.md) lists every search and its pattern rules.

## Read rows as JSON

The plain rows are separated by spaces, so a path with a space in it is hard to
split. `--json` prints each row as an array instead.

```console
$ ripscope --json -dL -3 greet
["main.c","main",5,"greet(\"world\");"]
$ ripscope --json -dL -3 greet | jq -r '"\(.[0]):\(.[2])"'
main.c:5
```

The [JSON interface](json.md) describes the fields, including what a row looks
like when a path or line is not valid UTF-8.

## Fail a job when a function is still called

A search exits with status 0 whether or not it finds anything, so test the output.

```sh
test -z "$(ripscope -dL -3 old_api)"
```

The job fails while any caller of `old_api` remains.

## Tell an empty answer from a mistake

Ripscope exits with status 1 when it cannot run the search, for example when
there is no index:

```console
$ ripscope -dL -1 main
ripscope: no index at ripscope.db
```

A pattern it cannot use is different. As in cscope, the search prints no rows and
exits with status 0, and the reason goes to standard error:

```console
$ ripscope -dL -1 'gre('
ripscope: invalid query pattern: invalid syntax
```

Treat anything on standard error as a reason to stop. That also catches warnings,
such as a file a text search could not read.

## Ask many questions

Starting Ripscope once is faster than starting it for every search. A session
reads one search per line, the field digit followed by the pattern, and `q` ends
it.

```console
$ printf '1greet\n3greet\nq\n' | ripscope --json -dl
{"type":"ready"}
{"type":"result","count":1}
["greet.c","greet",6,"void greet(const char *name)"]
{"type":"ready"}
{"type":"result","count":1}
["main.c","main",5,"greet(\"world\");"]
{"type":"ready"}
```

[Sessions](sessions.md) and the [JSON interface](json.md#sessions) describe the
commands and frames.
