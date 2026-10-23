// Runs tb with a VTR trace in pipeline.vtr. +nosignals opens the file without
// registering the model's signals: the recording holds clocks, transactions and log.
#include "Vtb.h"
#include "verilated.h"
#include "verilated_vtr_c.h"
#include <string_view>

int main(int argc, char** argv) {
    VerilatedContext context;
    context.commandArgs(argc, argv);
    context.traceEverOn(true);
    const bool noSignals = context.commandArgsPlusMatch("nosignals")[0];
    Vtb model{&context};
    vtr::WriterOptions options;
    if (argc > 1 && std::string_view{argv[1]} == "inline") options.mode = vtr::ExecutionMode::inline_;
    VerilatedVtrC trace{options};
    if (!noSignals) model.trace(&trace, 99);
    trace.open("pipeline.vtr");
    while (!context.gotFinish()) {
        model.eval();
        trace.dump(context.time());
        if (!model.eventsPending()) break;
        context.time(model.nextTimeSlot());
    }
    model.final();
    trace.close();
}
