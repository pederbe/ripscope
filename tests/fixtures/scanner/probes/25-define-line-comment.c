#  if ! defined(STATIC_X)
#    if defined(TESTING)
#      define STATIC_X // empty i.e. not static
#    else
#      define STATIC_X static
#    endif
#  endif
int after_directives;
