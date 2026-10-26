package org.cmusatyalab.uriel.example;

import android.os.Bundle;
import android.widget.Button;
import android.widget.EditText;
import android.widget.TextView;

import androidx.appcompat.app.AppCompatActivity;

import java.io.ByteArrayOutputStream;
import java.io.InputStream;
import java.nio.charset.StandardCharsets;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;

import uniffi.uriel_android.Client;
import uniffi.uriel_android.Event;
import uniffi.uriel_android.EventKind;
import uniffi.uriel_android.Producer;
import uniffi.uriel_android.RemoteServer;
import uniffi.uriel_android.Subscription;

/**
 * Sends text to an engine on a remote Uriel server, and shows what happens to it.
 *
 * <p>The server's certificate is read from the asset {@code server-certificate.der}: copy the
 * {@code certificate.der} that the server keeps in its identity directory there.
 */
public class MainActivity extends AppCompatActivity {
    private static final String CERTIFICATE_ASSET = "server-certificate.der";
    private static final String CLIENT_NAME = "android-example";
    private static final String PRODUCER_NAME = "camera";
    private static final String FLOW_NAME = "detect";
    private static final long MAX_INPUT_SIZE = 4096;
    private static final int READ_CHUNK_SIZE = 4096;
    /** How long the events thread waits for an event before it checks whether to stop. */
    private static final long EVENT_WAIT_MS = 500;
    private static final long EVENTS_THREAD_STOP_MS = 2000;

    /** Runs the calls that block, off the main thread. */
    private final ExecutorService background = Executors.newSingleThreadExecutor();

    private TextView log;
    private Button send;
    private EditText message;

    private Client client;
    private Producer producer;
    private Subscription events;
    private Thread eventsThread;
    private volatile boolean reading;

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        setContentView(R.layout.activity_main);
        log = findViewById(R.id.log);
        send = findViewById(R.id.send);
        message = findViewById(R.id.message);
        EditText serverAddress = findViewById(R.id.server_address);
        EditText engine = findViewById(R.id.engine);

        findViewById(R.id.connect).setOnClickListener(view -> {
            String address = serverAddress.getText().toString();
            String engineName = engine.getText().toString();
            background.execute(() -> connect(address, engineName));
        });
        send.setOnClickListener(view -> {
            String text = message.getText().toString();
            background.execute(() -> send(text));
        });
    }

    @Override
    protected void onDestroy() {
        super.onDestroy();
        background.execute(this::disconnect);
        background.shutdown();
    }

    /** Connects to the server, and starts reading the events about the inputs. */
    private void connect(String address, String engine) {
        disconnect();
        try {
            // The client trusts this certificate only.
            byte[] certificate;
            try (InputStream in = getAssets().open(CERTIFICATE_ASSET)) {
                ByteArrayOutputStream bytes = new ByteArrayOutputStream();
                byte[] chunk = new byte[READ_CHUNK_SIZE];
                for (int n = in.read(chunk); n >= 0; n = in.read(chunk)) {
                    bytes.write(chunk, 0, n);
                }
                certificate = bytes.toByteArray();
            }
            // null: the default number of inputs in flight on each edge.
            RemoteServer server = new RemoteServer(address, certificate, null);
            client = new Client(CLIENT_NAME, server);
            producer = new Producer(PRODUCER_NAME, "text/plain", MAX_INPUT_SIZE);
            // The subscription exists before the client takes the first input, so it misses no event.
            events = client.addProducerWithEvents(PRODUCER_NAME);
            client.addFlow(FLOW_NAME, flowSpec(engine));
        } catch (Exception e) {
            disconnect();
            show("connect failed: " + e);
            return;
        }
        reading = true;
        eventsThread = new Thread(this::readEvents, "uriel-events");
        eventsThread.start();
        runOnUiThread(() -> send.setEnabled(true));
        show("connected to " + address);
    }

    private void send(String text) {
        if (producer == null) {
            return;
        }
        try {
            long seq = producer.send(text.getBytes(StandardCharsets.UTF_8));
            show("sent input " + seq);
        } catch (Exception e) {
            show("send failed: " + e);
        }
    }

    /** Shows each event about the inputs until {@link #disconnect} stops it. */
    private void readEvents() {
        Subscription subscription = events;
        while (reading) {
            try {
                // null when no event came within the wait.
                Event event = subscription.next(EVENT_WAIT_MS);
                if (event != null) {
                    show(describe(event));
                }
            } catch (Exception e) {
                show("events failed: " + e);
                return;
            }
        }
    }

    private static String describe(Event event) {
        if (event.getKind() == EventKind.RESULT) {
            String result = new String(event.getResult(), StandardCharsets.UTF_8);
            return "input " + event.getSeq() + ": " + event.getEngine() + " replied: " + result;
        }
        return "input " + event.getSeq() + ": " + event.getKind();
    }

    /** Stops reading events, and closes what {@link #connect} opened. */
    private void disconnect() {
        reading = false;
        if (eventsThread != null) {
            try {
                eventsThread.join(EVENTS_THREAD_STOP_MS);
            } catch (InterruptedException e) {
                Thread.currentThread().interrupt();
            }
            eventsThread = null;
        }
        // Removing the client also removes its flows.
        for (AutoCloseable object : new AutoCloseable[] {events, producer, client}) {
            if (object != null) {
                try {
                    object.close();
                } catch (Exception e) {
                    show("close failed: " + e);
                }
            }
        }
        events = null;
        producer = null;
        client = null;
        runOnUiThread(() -> send.setEnabled(false));
    }

    /** A flow from the producer to {@code engine}, which runs on the remote server. */
    private static String flowSpec(String engine) {
        return String.format(
                "[[producers]]\nname = \"%1$s\"\n"
                        + "[[engines]]\nname = \"%2$s\"\nlocation = \"remote\"\n"
                        + "[[edges]]\nfrom = \"%1$s\"\nto = \"%2$s\"\ntimeout = \"1s\"\n",
                PRODUCER_NAME, engine);
    }

    private void show(String line) {
        runOnUiThread(() -> log.append(line + "\n"));
    }
}
