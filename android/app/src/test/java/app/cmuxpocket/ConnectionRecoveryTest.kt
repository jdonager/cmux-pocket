package app.cmuxpocket

import androidx.lifecycle.viewModelScope
import app.cmuxpocket.transport.ConnectionStatus
import app.cmuxpocket.ui.AppSyncPhase
import app.cmuxpocket.ui.TerminalViewModel
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.cancel
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.test.setMain
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test

@OptIn(ExperimentalCoroutinesApi::class)
class ConnectionRecoveryTest {
    private val dispatcher = StandardTestDispatcher()

    @Before
    fun setUp() {
        Dispatchers.setMain(dispatcher)
    }

    @After
    fun tearDown() {
        Dispatchers.resetMain()
    }

    @Test
    fun invalidSavedProfileSurvivesStartupWithoutLosingTheError() = runTest {
        val viewModel = TerminalViewModel()
        try {
            // Startup auto-connect can run before the status collector's first dispatch.
            viewModel.connect("ws://gateway.example.test:8088", 8088, "test-token")
            runCurrent()

            assertTrue(viewModel.statusMessage.value.contains("use wss://"))
            assertEquals(ConnectionStatus.DISCONNECTED, viewModel.connectionStatus.value)
            assertEquals(AppSyncPhase.DISCONNECTED, viewModel.syncPhase.value)

            val error = viewModel.statusMessage.value
            advanceTimeBy(120_000)
            runCurrent()
            assertEquals(error, viewModel.statusMessage.value)
        } finally {
            viewModel.viewModelScope.cancel()
        }
    }

    @Test
    fun malformedUrlDoesNotLeaveManualConnectStuckConnecting() = runTest {
        val viewModel = TerminalViewModel()
        try {
            runCurrent()
            viewModel.connect("wss://[", 443, "test-token")
            runCurrent()

            assertEquals(ConnectionStatus.DISCONNECTED, viewModel.connectionStatus.value)
            assertEquals(AppSyncPhase.DISCONNECTED, viewModel.syncPhase.value)
            val error = viewModel.statusMessage.value
            assertTrue(error.isNotBlank())
            assertTrue(error != "Disconnected" && error != "Connecting...")

            advanceTimeBy(120_000)
            runCurrent()
            assertEquals(error, viewModel.statusMessage.value)
        } finally {
            viewModel.viewModelScope.cancel()
        }
    }
}
