package app.homebody.idea;

import com.google.gson.JsonElement;
import com.google.gson.JsonNull;
import com.google.gson.JsonObject;
import com.google.gson.JsonParser;

import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.net.StandardProtocolFamily;
import java.net.UnixDomainSocketAddress;
import java.nio.ByteBuffer;
import java.nio.channels.SocketChannel;
import java.nio.charset.StandardCharsets;
import java.nio.file.Path;

/**
 * Talks to the Homebody app running on this computer: one JSON object per line over the app's
 * private socket. Nothing here reaches the network.
 */
public final class HomebodyClient {
    private HomebodyClient() {}

    /** A sentence for the person, from Homebody or about reaching it. */
    public static final class Refused extends Exception {
        public Refused(String message) { super(message); }
    }

    static Path dataDir() {
        String override = System.getenv("HOMEBODY_DATA_DIR");
        if (override != null && !override.isEmpty()) return Path.of(override);
        String home = System.getProperty("user.home");
        String os = System.getProperty("os.name", "").toLowerCase();
        if (os.contains("mac")) return Path.of(home, "Library", "Application Support", "Homebody");
        String xdg = System.getenv("XDG_DATA_HOME");
        return xdg != null && !xdg.isEmpty() ? Path.of(xdg, "Homebody") : Path.of(home, ".local", "share", "Homebody");
    }

    /** One command, one answer. */
    public static JsonElement call(String cmd, JsonObject args) throws Refused {
        if (System.getProperty("os.name", "").toLowerCase().contains("win")) {
            throw new Refused("This plugin does not talk to Homebody on Windows yet.");
        }
        JsonObject req = new JsonObject();
        req.addProperty("cmd", cmd);
        req.add("args", args == null ? JsonNull.INSTANCE : args);
        byte[] line = (req + "\n").getBytes(StandardCharsets.UTF_8);
        try (SocketChannel ch = SocketChannel.open(StandardProtocolFamily.UNIX)) {
            ch.connect(UnixDomainSocketAddress.of(dataDir().resolve("control.sock")));
            ByteBuffer out = ByteBuffer.wrap(line);
            while (out.hasRemaining()) ch.write(out);
            ByteArrayOutputStream answer = new ByteArrayOutputStream();
            ByteBuffer in = ByteBuffer.allocate(64 * 1024);
            read:
            while (ch.read(in) > 0) {
                in.flip();
                while (in.hasRemaining()) {
                    byte b = in.get();
                    if (b == '\n') break read;
                    answer.write(b);
                }
                in.clear();
                if (answer.size() > 32 * 1024 * 1024) throw new Refused("Homebody answered with too much.");
            }
            JsonObject v = JsonParser.parseString(answer.toString(StandardCharsets.UTF_8)).getAsJsonObject();
            if (v.has("ok") && v.get("ok").getAsBoolean()) return v.has("result") ? v.get("result") : JsonNull.INSTANCE;
            throw new Refused(v.has("error") ? v.get("error").getAsString() : "Homebody refused.");
        } catch (IOException | RuntimeException e) {
            throw new Refused("Homebody is not running. Open the Homebody app.");
        }
    }

    static JsonObject args(Object... pairs) {
        JsonObject o = new JsonObject();
        for (int i = 0; i + 1 < pairs.length; i += 2) {
            Object v = pairs[i + 1];
            if (v instanceof Boolean b) o.addProperty((String) pairs[i], b);
            else if (v instanceof Number n) o.addProperty((String) pairs[i], n);
            else if (v != null) o.addProperty((String) pairs[i], v.toString());
        }
        return o;
    }

    /** The few words for the status bar. */
    public static String summary(JsonObject status) {
        if (status == null) return "Homebody: not running";
        int pending = status.has("pending") ? status.getAsJsonArray("pending").size() : 0;
        String ind = status.has("indicator") ? status.get("indicator").getAsString() : "";
        boolean locked = status.has("lockdown") && status.get("lockdown").getAsBoolean();
        boolean setup = status.has("setupNeeded") && status.get("setupNeeded").getAsBoolean();
        if (pending > 0) return pending == 1 ? "Homebody: 1 question" : "Homebody: " + pending + " questions";
        if (setup || ind.equals("attention")) return "Homebody: needs attention";
        if (locked) return "Homebody: everything blocked";
        if (ind.equals("off")) return "Homebody: protection off";
        if (ind.equals("watching")) return "Homebody: watching";
        return "Homebody: private";
    }

    static String text(JsonObject o, String key) {
        return o != null && o.has(key) && !o.get(key).isJsonNull() ? o.get(key).getAsString() : "";
    }
}
