#include <stdio.h>
#include "greet.h"

int greet_count;

void greet(const char *name)
{
    greet_count = greet_count + 1;
    printf("Hello, %s!\n", name);
}
