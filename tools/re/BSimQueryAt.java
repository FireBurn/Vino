// Headless BSim query: for each address argument, report the corpus functions most similar to
// the function containing it. Names travel from the binaries that kept their symbols (the macOS
// agent, the Windows dlidusb driver) to the stripped Linux DLM.
//
// Usage:
//   ghidra-headless <projdir> <Project> -process <binary> -noanalysis \
//       -scriptPath scripts/codec-re -postScript BSimQueryAt.java \
//       file:/home/fireburn/dlm-bsim/dlm 0x<addr> [0x<addr> ...]
//
//@category BSim

import java.net.URL;
import java.util.Iterator;

import ghidra.app.script.GhidraScript;
import ghidra.features.bsim.query.*;
import ghidra.features.bsim.query.description.*;
import ghidra.features.bsim.query.protocol.*;
import ghidra.program.model.address.Address;
import ghidra.program.model.listing.Function;

public class BSimQueryAt extends GhidraScript {

	private static final int MATCHES_PER_FUNC = 12;
	private static final double SIMILARITY_BOUND = 0.5;
	private static final double CONFIDENCE_BOUND = 0.0;

	@Override
	public void run() throws Exception {
		String[] args = getScriptArgs();
		if (currentProgram == null || args.length < 2) {
			println("usage: BSimQueryAt.java <db-url> 0xaddr [0xaddr ...]");
			return;
		}
		URL url = BSimClientFactory.deriveBSimURL(args[0]);
		try (FunctionDatabase database = BSimClientFactory.buildClient(url, false)) {
			if (!database.initialize()) {
				println("DB ERROR " + database.getLastError().message);
				return;
			}
			for (int i = 1; i < args.length; i++) {
				Address addr = toAddr(Long.decode(args[i]));
				Function func = getFunctionContaining(addr);
				if (func == null) {
					println("NO_FUNC_AT " + args[i]);
					continue;
				}
				println("=== QUERY " + func.getName() + " @ " + func.getEntryPoint() + " ===");
				GenSignatures gensig = new GenSignatures(false);
				try {
					gensig.setVectorFactory(database.getLSHVectorFactory());
					gensig.openProgram(currentProgram, null, null, null, null, null);
					DescriptionManager manager = gensig.getDescriptionManager();
					gensig.scanFunction(func);

					QueryNearest query = new QueryNearest();
					query.manage = manager;
					query.max = MATCHES_PER_FUNC;
					query.thresh = SIMILARITY_BOUND;
					query.signifthresh = CONFIDENCE_BOUND;

					ResponseNearest response = query.execute(database);
					if (response == null) {
						println("QUERY ERROR " + database.getLastError().message);
						continue;
					}
					for (SimilarityResult sim : response.result) {
						Iterator<SimilarityNote> subiter = sim.iterator();
						while (subiter.hasNext()) {
							SimilarityNote note = subiter.next();
							FunctionDescription fdesc = note.getFunctionDescription();
							println(String.format("  %.4f  %-28s %s",
								note.getSimilarity(),
								fdesc.getExecutableRecord().getNameExec(),
								fdesc.getFunctionName()));
						}
					}
				}
				finally {
					gensig.dispose();
				}
			}
		}
	}
}
