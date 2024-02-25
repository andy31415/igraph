# Include dependendcy viewer

This program generates [dot](https://graphviz.org/doc/info/lang.html) graph
descriptions for include headers for c/c++.

The intent is to visualize dependencies among files.

## Functionality

- select what files to parse
- define how to group to viewed files
- abiltiy to [compile database](https://clang.llvm.org/docs/JSONCompilationDatabase.html) 
  files to figure out imports. These files can be generated by gn/cmake/other tools.
- abiltiy to use [gn](https://gn.googlesource.com/gn/) to auto-group headers

## Usage

```sh
# Simple run using a configuration database
# if no `output` is provided, the output will go to standard out
igraph -c configfile.txt -o outfile.dot

# You should generate the graph using graphviz/dot
# For example for the above `outfile.dot`:
dot -T svg -o outfile.svg outfile.dot

# If you install watchexec, you can setup a pipeline like:
watchexec -e txt -- "igraph -c cfg.txt -o out.dot && dot -Tsvg -o out.svg out.dot"
```

## Configuration file format

Here is an example configuration file with comments

```
# Comments start with `#` and last to the end of the line

# Variables are declared first and you can nest variables
# Expansion is specifically `${name}` (this is not shell, so `$name` will not work)
SOURCE_ROOT=/some/path/to/source
OUTPUT_ROOT=${SOURCE_ROOT}/build/out

# The input section describes what files are to be parsed.
#   - what include path  should be searched for `#include "foo.h"`
#   - what files to be parsed using glob rules
input {
    # You may include a compile_commands database which will parse
    # includes (find `-I` arguments to a compiler)
    includes from compiledb ${OUTPUT_ROOT}/compile_commands.json
    
    # You may also manually include single directories
    include_dir ${SOURCE_ROOT}/includes/test
    include_dir /third/party/lib

    # Globs are generally including all files. program filters
    # out based on extensions (h, hpp, c, cpp, cxx, cc)
    glob ${SOURCE_ROOT}/src/lib1/**/*
    glob ${SOURCE_ROOT}/src/lib2/**/*
}

# The graph section defines how to setup the graph.
graph {
   # The tool works with absolute paths when parsing includes
   # the `map` section describes how to shorten typically long
   # absolute paths like `/home/user/devel/some/path/....`
   map {
      # Replace some long poath with a short prefix
      ${SOURCE_ROOT}/src/lib1 => first::
      ${SOURCE_ROOT}/src/lib2 => second::

      # This defines what items to actually keep in the output. Not
      # all includes are kept as they would be generally too large.
      #
      # Only prefix-paths are used here
      keep first::
      keep second::

      # Explicitly remove some of the kept items
      drop first::tests/
      drop second::support/library
   }

   # The group section defines how the graph should place
   # things together for easy dependency view.
   # Group logic:
   #   - they must be in order of application
   #   - first group wins (a file belongs to one group only)
   group {
      # This loads build targets from GN:
      #  - compile_root is where the `gn` too will be pointed to
      #  - target defines what GN target to grab `sources` from
      #  - sources will translate gn paths of `//foo` into absolute
      #    system paths
      gn root ${OUTPUT_ROOT} target //src/app/* sources ${SOURCE_ROOT}

      # Groups can be manually defined to group some files
      manual group-name-here {
         # Grouping is done by mapped names
         first::platform/Header.h
         first::platform/Src.cpp
         first::Something.cc
       }
      
       # Optional instructions to ensure headers and sources
       # are grouped together (as they are generally included in
       # the same compilation unit)
       group_source_header
   }

   # If zoom is non-empty it generates a separate area
   # with the specified "groups" expanded and viewing individual
   # members and dependencies
   zoom {
     # The zoom takes the group name argument, which could
     # be the GN group name or the manual group name
     //src/library:support
     group-name-here
     
     # Focus prefix means to focus on determining dependencies
     # in and out of the specified group(s).
     #
     # Dependences from other zoomed items will only be show
     # if internal or they start/end in a focused zoo group
     focus: //src/library:foo
     focus: //src/something/else:else
   }
}
```
