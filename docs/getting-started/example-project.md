# Example project

Three small files are enough to try definitions, callers, callees, assignments and
includes, and to compare your output with the answers shown here.
[Install Ripscope](installation.md) first, then create these files in a new
directory.

```text
demo/
├── greet.c
├── main.c
└── include/
    └── greet.h
```

```c
/* include/greet.h */
void greet(const char *name);
extern int greet_count;

/* greet.c */
#include <stdio.h>
#include "greet.h"

int greet_count;

void greet(const char *name)
{
    greet_count = greet_count + 1;
    printf("Hello, %s!\n", name);
}

/* main.c */
#include "greet.h"

int main(void)
{
    greet("world");
    return 0;
}
```

## 1. Build the index

From the project directory:

```console
$ ripscope -bk -I include
```

It prints nothing when it succeeds, and `ripscope.db` appears in the directory.

- `-b` builds or updates the index, as in cscope. With no file arguments, Ripscope uses
  `cscope.files` if it can be read, otherwise the source files in the current directory with
  cscope's suffixes (`.c`, `.h` and [others](../guide/building.md#directory-discovery)).
  Add `-R` to include subdirectories.
- `-k` leaves out the system include directory, as in cscope. Without it, headers such
  as `stdio.h` are indexed too.
- `-I include` lets `#include "greet.h"` resolve, so `include/greet.h` is indexed too.
- `-f INDEX` names another index file than `ripscope.db`.

## 2. Ask questions

Each query names a **field** (`-0` to `-9`) and a pattern. `-L` prints matching rows as
`file context line text`.

```console
$ ripscope -L -1 greet          # where is greet defined?
greet.c greet 6 void greet(const char *name)

$ ripscope -L -3 greet          # who calls greet?
main.c main 5 greet("world");

$ ripscope -L -2 main           # what does main call?
main.c greet 5 greet("world");

$ ripscope -L -9 greet_count    # where is greet_count assigned?
greet.c greet 8 greet_count = greet_count + 1;

$ ripscope -L -8 greet.h        # who includes greet.h?
greet.c <global> 2 #include "greet.h"
main.c <global> 1 #include "greet.h"
```

The second column is the **context**: the enclosing function, `<global>` for file scope,
or `<unknown>` where it does not apply. See [Queries](../guide/queries.md) for all the
fields.

## 3. Keep it fresh

`-L` brings the index up to date before answering, analysing only files that changed.
Add a new caller and ask again:

```console
$ echo 'void helper(void) { greet("x"); }' >> main.c
$ ripscope -L -3 greet
main.c main 5 greet("world");
main.c helper 8 void helper(void ) { greet("x"); }
```

Use `-dL` to query the index as it is. Stored queries work even when the source files
are gone.

## 4. Use it from your editor

Editors keep a Ripscope [session](../guide/sessions.md) open, one process that
answers many queries. Vim sends it one query per line, like this:

```console
$ printf '1greet\n3greet\nq\n' | ripscope -l
>> cscope: 1 lines
greet.c greet 6 void greet(const char *name)
>> cscope: 2 lines
main.c main 5 greet("world");
main.c helper 8 void helper(void ) { greet("x"); }
>>
```

You rarely type this yourself, though a
[script that asks many questions](../guide/scripts.md#ask-many-questions) can. With [Vim](../editors/vim.md),
[Neovim](../editors/neovim.md) or [Emacs](../editors/emacs.md) set up, open
`main.c` and search there instead. The editors keep their own index of the project in
your cache directory. Save an edit and search again to see it. Each editor reuses its
session between searches.

## Exact output

Paths are printed as they are, spaces included, as cscope prints them. Add `--json` to
get one JSON array per row, which carries every byte:

```console
$ ripscope --json -L -3 greet
["main.c","main",5,"greet(\"world\");"]
["main.c","helper",8,"void helper(void ) { greet(\"x\"); }"]
```

See the [JSON interface](../guide/json.md).
