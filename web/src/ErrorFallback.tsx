import type { FallbackProps } from 'react-error-boundary'
import { RefreshCw, TriangleAlert } from 'lucide-react'
import { Alert, AlertTitle, AlertDescription } from './components/ui/alert'
import { Button } from './components/ui/button'

export const ErrorFallback = ({ error, resetErrorBoundary }: FallbackProps) => (
  <div className="min-h-screen bg-background flex items-center justify-center p-4">
    <div className="w-full max-w-md">
      <Alert variant="destructive" className="mb-6">
        <TriangleAlert />
        <AlertTitle>Runtime error</AlertTitle>
        <AlertDescription>Something went wrong while running the engine.</AlertDescription>
      </Alert>

      <pre className="text-xs text-destructive bg-muted/50 p-3 mb-6 rounded border overflow-auto max-h-32">
        {error instanceof Error ? error.message : String(error)}
      </pre>

      <Button onClick={resetErrorBoundary} className="w-full" variant="outline">
        <RefreshCw />
        Try Again
      </Button>
    </div>
  </div>
)
