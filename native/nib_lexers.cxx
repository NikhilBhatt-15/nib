// Static Lexilla catalogue for the lexers Nib ships, replacing Lexilla.cxx
// (which references every lexer in the library).

#include <cstring>
#include <vector>
#include <initializer_list>

#include "ILexer.h"
#include "LexerModule.h"

using namespace Lexilla;

extern const LexerModule lmBash, lmBatch, lmCPP, lmCss, lmCmake, lmDart, lmDiff, lmHTML, lmXML,
    lmPHPSCRIPT, lmJSON, lmLua, lmMake, lmMarkdown, lmNull, lmPerl, lmPowerShell, lmProps,
    lmPython, lmRegistry, lmRuby, lmRust, lmSQL, lmTOML, lmVB, lmVBScript, lmYAML, lmZig;

static const LexerModule *const modules[] = {
    &lmBash, &lmBatch, &lmCPP, &lmCss, &lmCmake, &lmDart, &lmDiff, &lmHTML, &lmXML,
    &lmPHPSCRIPT, &lmJSON, &lmLua, &lmMake, &lmMarkdown, &lmNull, &lmPerl, &lmPowerShell, &lmProps,
    &lmPython, &lmRegistry, &lmRuby, &lmRust, &lmSQL, &lmTOML, &lmVB, &lmVBScript, &lmYAML, &lmZig,
};

// Returns an ILexer5* for SCI_SETILEXER, or null for an unknown name.
extern "C" void *nib_create_lexer(const char *name) {
    for (const LexerModule *lm : modules) {
        if (lm->languageName && std::strcmp(lm->languageName, name) == 0) {
            return lm->Create();
        }
    }
    return nullptr;
}
