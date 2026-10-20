%{
#include "lexer.h"
int lex_global;
%}
%%
[0-9]+   { return number(yytext); }
%%
int helper(void) { return 0; }
