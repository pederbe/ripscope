%{
int yacc_global;
%}
%token NUM
%%
expr: NUM { $$ = make($1); }
    ;
%%
int yyerror(char *s) { return report(s); }
